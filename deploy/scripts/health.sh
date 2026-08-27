#!/usr/bin/env bash
# health.sh — read-only crawler health collector for cross-run comparison.
#
# Usage:
#   ./health.sh                     # compact human summary (last 15 min)
#   ./health.sh --all               # minute-bucketed full metric history
#   ./health.sh --json              # machine-readable summary (JSON)
#
# Optional:
#   --window 15|30|60               # analysis window in minutes (default 15)
#   --session <epoch/ts>            # explicit session start (default: latest)
#   --since "2026-08-26 17:00:00"   # overrides window with absolute start
#   --no-logs                       # skip JSONL log anomaly scanning
#   --host zerone                   # target host (default zerone)
#
# All rates derive from PostgreSQL `metrics` (session-scoped cumulative
# deltas). JSONL is used ONLY for anomaly/trace context, never for rates.
# Read-only: no writes/DDL to the database.
set -euo pipefail

HOST="zerone"
WINDOW=15
ALL_MODE=0
JSON_MODE=0
NO_LOGS=0
SESSION_OVERRIDE=""
SINCE_OVERRIDE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --all) ALL_MODE=1 ;;
        --json) JSON_MODE=1 ;;
        --window) WINDOW="$2"; shift ;;
        --session) SESSION_OVERRIDE="$2"; shift ;;
        --since) SINCE_OVERRIDE="$2"; shift ;;
        --no-logs) NO_LOGS=1 ;;
        --host) HOST="$2"; shift ;;
        -*)
            if [ -f "$(dirname "$0")/.health-hosts" ]; then
                if grep -qx "$1" "$(dirname "$0")/.health-hosts"; then HOST="${1#--}"; fi
            fi
            echo "unknown arg: $1" >&2; exit 1 ;;
        *) HOST="$1" ;;
    esac
    shift
done

SSH_KEY="${HOME}/.ssh/zerone"
SSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no"
LOG_DIR="/home/ubuntu/gaia-data/logs"
TAB="$(printf '\t')"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Pass the few needed knobs through the environment created by ssh heredoc args.
REMOTE_SCRIPT="$(cat <<'RS'
set -euo pipefail
WINDOW="$1"
SESSION_OVERRIDE="$2"
SINCE_OVERRIDE="$3"
NO_LOGS="$4"

DB_CONN="postgres://crawler:83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b@100.87.194.112:5432/craw?sslmode=disable"
TAB="$(printf '\t')"
PSQL=(docker run --rm --network host postgres:16 psql "$DB_CONN" -t -A -F "$TAB" -c)

q() { "${PSQL[@]}" "$1"; }

# Session start
if [ -n "$SESSION_OVERRIDE" ] && [ "$SESSION_OVERRIDE" != "__none__" ]; then
    SESSION_TS="$SESSION_OVERRIDE"
else
    SESSION_TS="$(q "SELECT max(ts) FROM metrics WHERE metric_name='_session_start';")"
fi

# Window start timestamp string (UTC, for log scanning) + SQL bound.
if [ -n "$SINCE_OVERRIDE" ] && [ "$SINCE_OVERRIDE" != "__none__" ]; then
    FROM_TS="$SINCE_OVERRIDE"
    WSTART_SQL="'$SINCE_OVERRIDE'::timestamptz"
else
    FROM_TS="$(q "SELECT to_char(now() - interval '${WINDOW} minutes', 'YYYY-MM-DD HH24:MI:SS.US');")"
    WSTART_SQL="now() - interval '${WINDOW} minutes'"
fi

echo "=== META ==="
printf  "session\t%s\n" "$SESSION_TS"
printf  "window_min\t%s\n" "$WINDOW"
printf  "from_ts\t%s\n" "$FROM_TS"

echo "=== DELTAS ==="
q "WITH s AS (SELECT max(ts) AS start_ts FROM metrics WHERE metric_name='_session_start'),
      ws AS (SELECT GREATEST(s.start_ts, $WSTART_SQL) AS wstart FROM s)
   SELECT metric_name, min(metric_value), max(metric_value)
   FROM metrics, ws
   WHERE ts >= ws.wstart
   GROUP BY metric_name
   ORDER BY metric_name;"

echo "=== GAUGES ==="
q "WITH s AS (SELECT max(ts) AS start_ts FROM metrics WHERE metric_name='_session_start'),
      latest AS (SELECT max(ts) AS t FROM metrics WHERE ts >= (SELECT start_ts FROM s))
   SELECT metric_name, metric_value
   FROM metrics
   WHERE ts = (SELECT t FROM latest)
     AND metric_name IN ('verify_channel_depth','verify_channel_depth_max',
                         'fresh_channel_depth','fresh_channel_depth_max')
   ORDER BY metric_name;"

echo "=== JOBS ==="
q "SELECT status, count(*) FROM verification_jobs GROUP BY status ORDER BY status;"

echo "=== TORRENTS ==="
q "SELECT count(*) FILTER (WHERE verified_at > now() - interval '1 hour') AS v1h,
          count(*) FILTER (WHERE verified_at > now() - interval '24 hours') AS v24h
   FROM torrents;"

echo "=== CONTAINER ==="
docker inspect gaia-crawler --format 'image={{.Config.Image}}uptime={{.State.StartedAt}}restarts={{.RestartCount}}' 2>/dev/null || echo "container inspect failed"

echo "=== DOCKER_PS ==="
docker ps --filter name=gaia-crawler --format '{{.Names}}${{.Status}}${{.Image}}' 2>/dev/null || true

if [ "$NO_LOGS" != "1" ]; then
echo "=== EFFECTIVE_CONFIG ==="
ls -t /home/ubuntu/gaia-data/logs/crawler-*.jsonl 2>/dev/null | head -3 | xargs grep -h '"effective config"' 2>/dev/null | tail -1 || echo "(none)"

echo "=== LOG_ANOMALIES ==="
python3 - /home/ubuntu/gaia-data/logs "$FROM_TS" <<'PY'
import json, os, sys, datetime
logdir, frm = sys.argv[1], sys.argv[2]
frm = frm.strip()
frm_dt = None
for fmt in ("%Y-%m-%d %H:%M:%S.%f", "%Y-%m-%d %H:%M:%S"):
    try:
        frm_dt = datetime.datetime.strptime(frm, fmt)
        break
    except ValueError:
        continue
if frm_dt is None:
    print("ERR\tbad from_ts")
    sys.exit(0)
cnt = {"slow":0,"warn":0,"error":0,"panic":0,"log_dropped":0,"dropped":0}
samples = []
for fn in sorted(os.listdir(logdir)):
    if not (fn.startswith("crawler-") and fn.endswith(".jsonl")): continue
    p = os.path.join(logdir, fn)
    if not os.path.isfile(p): continue
    try:
        with open(p, "r") as f:
            for line in f:
                try: e = json.loads(line)
                except Exception: continue
                ts = e.get("ts","")
                if not ts: continue
                try:
                    dt = datetime.datetime.fromisoformat(ts.replace("Z","+00:00"))
                    dt = dt.astimezone(datetime.timezone.utc).replace(tzinfo=None)
                except Exception: continue
                if dt < frm_dt: continue
                msg = e.get("message","")
                lvl = e.get("level","")
                if "slow statement" in msg:
                    cnt["slow"] += 1
                    if len(samples)<20: samples.append(line.rstrip())
                if lvl == "warn": cnt["warn"] += 1
                if lvl == "error":
                    cnt["error"] += 1
                    if len(samples)<20: samples.append(line.rstrip())
                if "panic" in msg or lvl == "panic":
                    cnt["panic"] += 1
                    if len(samples)<20: samples.append(line.rstrip())
                if e.get("log_dropped", 0): cnt["log_dropped"] += 1
                if "dropped" in msg and "log_dropped" not in msg:
                    cnt["dropped"] += 1
    except Exception:
        pass
print("slow_statements\t%d" % cnt["slow"])
print("warn\t%d" % cnt["warn"])
print("error\t%d" % cnt["error"])
print("panic\t%d" % cnt["panic"])
print("log_dropped\t%d" % cnt["log_dropped"])
print("dropped_msgs\t%d" % cnt["dropped"])
for s in samples:
    print("SAMPLE\t" + s)
PY
fi

echo "=== BUCKETS ==="
q "WITH s AS (SELECT max(ts) AS start_ts FROM metrics WHERE metric_name='_session_start')
   SELECT metric_name, to_char(date_trunc('minute', ts), 'MM-DD HH24:MI') AS bucket, max(metric_value)
   FROM metrics
   WHERE ts >= GREATEST((SELECT start_ts FROM s), $WSTART_SQL)
   GROUP BY metric_name, bucket
   ORDER BY metric_name, bucket;"
RS
)"

# ── Run remote capture ──
# NOTE: ssh drops empty args, so use sentinels for unset values.
SESS_ARG="${SESSION_OVERRIDE:-__none__}"
SINCE_ARG="${SINCE_OVERRIDE:-__none__}"
RAW="$($SSH "$HOST" "bash -s" "$WINDOW" "$SESS_ARG" "$SINCE_ARG" "$NO_LOGS" <<<"$REMOTE_SCRIPT" 2>/dev/null)" || {
    echo "ERROR: remote capture failed" >&2
    exit 1
}

echo "$RAW" > "$TMP/raw.txt"
if ! grep -q '^=== DELTAS ===' "$TMP/raw.txt"; then
    echo "ERROR: no DELTAS section in capture." >&2
    head -40 "$TMP/raw.txt" >&2
    exit 1
fi

PY_MODE="summary"
[ "$ALL_MODE" = "1" ] && PY_MODE="all"
[ "$JSON_MODE" = "1" ] && PY_MODE="json"

FMT="$(python3 - "$TMP/raw.txt" "$PY_MODE" <<'PY'
import json, sys, collections, math

raw_path, mode = sys.argv[1], sys.argv[2]
with open(raw_path) as f:
    lines = f.read().split("\n")

sections = collections.OrderedDict()
cur = None
for ln in lines:
    if ln.startswith("=== ") and ln.endswith(" ==="):
        cur = ln.strip("= ").strip()
        sections[cur] = []
    elif cur is not None:
        sections[cur].append(ln)

def tab_rows(key):
    out = []
    for ln in sections.get(key, []):
        if not ln.strip(): continue
        out.append(ln.split("\t"))
    return out

deltas = {}
for row in tab_rows("DELTAS"):
    if len(row) >= 3:
        try:
            deltas[row[0]] = (float(row[1]), float(row[2]))
        except ValueError:
            pass

gauges = {}
for row in tab_rows("GAUGES"):
    if len(row) >= 2:
        try:
            gauges[row[0]] = float(row[1])
        except ValueError:
            pass

jobs = {}
for row in tab_rows("JOBS"):
    if len(row) >= 2:
        try: jobs[row[0]] = int(row[1])
        except ValueError: pass

meta = {}
for row in tab_rows("META"):
    if len(row) >= 2:
        meta[row[0]] = row[1]

torrents = {"v1h": None, "v24h": None}
for row in tab_rows("TORRENTS"):
    if row:
        try:
            torrents["v1h"] = int(row[0])
            torrents["v24h"] = int(row[1])
        except Exception:
            pass

container = {}
for row in tab_rows("CONTAINER"):
    if row:
        val = row[0]
        import re
        m = re.match(r"image=(.+?)uptime=(.+?)restarts=(.*)$", val)
        if m:
            container["image"], container["uptime"], container["restarts"] = m.groups()

docker_rows = tab_rows("DOCKER_PS")
docker_ps = "\t".join(docker_rows[0]) if docker_rows else ""

config = sections.get("EFFECTIVE_CONFIG", [])
effcfg = config[0] if config else ""

anom = {}
samples = []
for row in tab_rows("LOG_ANOMALIES"):
    if len(row) >= 2:
        if row[0] == "SAMPLE":
            samples.append(row[1])
        elif row[0] != "ERR":
            try: anom[row[0]] = int(row[1])
            except ValueError: pass

def dlt(name):
    if name not in deltas: return None
    lo, hi = deltas[name]
    return hi - lo

def rate(name):
    if name not in deltas: return None
    lo, hi = deltas[name]
    win = float(meta.get("window_min", "15"))
    return (hi - lo) * (60.0 / win)

CHANNEL_CAP = 65536
flags = {"critical": [], "warning": [], "info": []}

for ch in ("verify_channel_depth", "fresh_channel_depth"):
    v = gauges.get(ch)
    if v is not None and v >= CHANNEL_CAP * 0.95:
        flags["critical"].append(f"{ch}={int(v)} near saturation ({CHANNEL_CAP})")
    elif v is not None and v >= CHANNEL_CAP * 0.5:
        flags["warning"].append(f"{ch}={int(v)} elevated")

for m in ("fresh_channel_dropped", "harvest_try_send_dropped", "scheduler_send_blocked"):
    r = dlt(m)
    if r and r > 0:
        flags["critical"].append(f"{m}={int(r)} (drops>0)")
sb = dlt("scheduler_skipped_backpressure")
if sb and sb > 0:
    flags["info"].append(f"scheduler_skipped_backpressure={int(sb)}")

pend = jobs.get("pending", 0); fail = jobs.get("failed", 0); ver = jobs.get("verifying", 0)
if pend + fail > 0 and ver == 0:
    flags["critical"].append(f"STARVATION: pending={pend} failed={fail} but verifying=0")

vr = rate("verify_success")
if vr is not None and vr < 1000:
    flags["warning"].append(f"verify_success/hr low ({vr:.0f})")
if anom.get("slow", 0) > 0:
    flags["warning"].append(f"slow statements in window: {anom['slow']}")
if anom.get("panic", 0) > 0:
    flags["critical"].append(f"panic in window: {anom['panic']}")
if anom.get("error", 0) > 0:
    flags["warning"].append(f"error log lines: {anom['error']}")
if anom.get("log_dropped", 0) > 0:
    flags["warning"].append(f"log_dropped in window: {anom['log_dropped']}")

def fmt_r(r):
    if r is None: return "-"
    if abs(r) >= 10000: return f"{r/1000:.0f}k"
    if abs(r) >= 1000: return f"{r/1000:.1f}k"
    return f"{r:,.0f}"

if mode == "json":
    out = {
        "meta": meta,
        "deltas": {k: (round(a,6), round(b,6)) for k,(a,b) in deltas.items()},
        "gauges": gauges,
        "rates": {k: (round(v,3) if v is not None else None) for k,v in
                  {n: rate(n) for n in deltas}.items()},
        "jobs": jobs,
        "torrents": torrents,
        "container": container,
        "docker_ps": docker_ps,
        "effective_config": effcfg,
        "log_anomalies": anom,
        "log_samples": samples[:10],
        "flags": flags,
    }
    print(json.dumps(out, indent=2))
    sys.exit(0)

if mode == "all":
    buckets = collections.OrderedDict()
    for row in tab_rows("BUCKETS"):
        if len(row) != 3: continue
        name, bucket, val = row[0], row[1], row[2]
        try: v = float(val)
        except ValueError: continue
        buckets.setdefault(bucket, {})[name] = v
    bt = list(buckets.keys())
    if not bt:
        print("(no bucketed data)")
        sys.exit(0)
    # pad each column to equal width based on its own max cell length
    names = sorted({n for b in buckets.values() for n in b})
    name_w = max([len(n) for n in names] + [30])
    widths = []
    for b in bt:
        cells = []
        for n in names:
            v = buckets[b].get(n)
            cells.append(str(int(v)) if v is not None else "-")
        widths.append(max(len(c) for c in cells) if cells else 6)
    def padw(s, w):
        return s.rjust(w) if len(s) < w else s
    print("metric".ljust(name_w) + "".join(padw(b.split()[1], w) for b, w in zip(bt, widths)))
    for name in names:
        prevv = None
        outvals = []
        for b in bt:
            v = buckets[b].get(name)
            if v is None:
                outvals.append("-")
            elif prevv is None:
                outvals.append(".")
            elif v >= prevv:
                outvals.append(f"{v-prevv:.0f}")
            else:
                outvals.append(f"{v:.0f}")
            prevv = v if v is not None else prevv
        print(name.ljust(name_w) + "".join(padw(v, w) for v, w in zip(outvals, widths)))
    sys.exit(0)

# HUMAN SUMMARY
print()
print(f"GAIA CRAWLER HEALTH  (window={meta.get('window_min','?')}m)  session_start={meta.get('session','?')}")
print(f"  image    : {container.get('image','?')}  restarts={container.get('restarts','?')}  uptime={container.get('uptime','?')}")
if docker_ps:
    print(f"  docker   : {docker_ps}")
if effcfg:
    import re
    keys = ["source_deadline_ms","source_max_queries","global_fetch_limit","pipeline_limit","connect_deadline_ms","race_peers",
            "tcp_timeout_secs","utp_timeout_secs","metadata_timeout_secs",
            "max_conns_per_ip","no_peers_terminal_on_first","fresh_channel_capacity"]
    vals = []
    for k in keys:
        m = re.search(f'"{k}":(\\d+)', effcfg)
        if m: vals.append(f"{k}={m.group(1)}")
    print("  config   : " + "  ".join(vals))
print()

def group(title, names):
    print(f"-- {title} --")
    for n in names:
        r = rate(n); d = dlt(n); g = gauges.get(n)
        if r is None and d is None and g is None: continue
        print(f"  {n:<40} rate={fmt_r(r):>9}  dlt={fmt_r(d):>8}  gauge={f'{g:,.0f}' if g is not None else '-':>8}")
    print()

group("CHANNELS", ["verify_channel_depth","fresh_channel_depth","verify_channel_depth_max",
                   "fresh_channel_depth_max","harvest_try_send_dropped","fresh_channel_dropped",
                   "scheduler_send_blocked","scheduler_skipped_backpressure"])
group("THROUGHPUT", ["verify_attempts","verify_success","verify_fail",
                     "fetch_attempts","unique_infohashes","tcp_metadata_ok","utp_metadata_ok"])
group("SOURCE", ["source_queries","source_responses","source_peers_returned",
                  "source_no_peers","source_timeout","source_deadline_hits",
                  "source_filtered_by_cache"])
group("SCHEDULER", ["scheduler_claims","scheduler_claimed_fresh","scheduler_claimed_retry"])
group("CONNECT", ["tcp_attempts","tcp_connect_ok","utp_attempts","utp_connect_ok",
                  "fetch_connect_timeout","fetch_connect_io"])
group("HARVEST/DHT", ["infohashes_harvested","inbound_get_peers","inbound_announce_peer",
                      "outbound_queries","outbound_timeouts"])

print("-- JOBS / DB --")
print(f"  jobs     : pending={jobs.get('pending',0)} verifying={jobs.get('verifying',0)} "
      f"failed={jobs.get('failed',0)} dead={jobs.get('dead',0)}")
print(f"  torrents : 1h={torrents.get('v1h','?')}  24h={torrents.get('v24h','?')}")
print(f"  log      : slow={anom.get('slow',0)} warn={anom.get('warn',0)} error={anom.get('error',0)} "
      f"panic={anom.get('panic',0)} log_dropped={anom.get('log_dropped',0)}")
print()

tr = rate("tcp_connect_ok"); ta = rate("tcp_attempts")
if tr is not None and ta:
    print(f"  CONNECT  : TCP ok/attempt={tr/ta*100:.1f}%", end="")
    mr = rate("tcp_metadata_ok")
    if mr is not None and tr:
        print(f"   metadata/tcp_ok={mr/tr*100:.1f}%", end="")
    ur = rate("utp_connect_ok"); ua = rate("utp_attempts")
    if ur is not None and ua:
        print(f"   uTP ok/attempt={ur/ua*100:.1f}%", end="")
    print()
print()

if flags["critical"] or flags["warning"] or flags["info"]:
    print("-- FLAGS --")
    for sev in ("critical","warning","info"):
        for fg in flags[sev]:
            print(f"  [{sev:>8}] {fg}")
    print()

if samples:
    print("-- LOG SAMPLES (window) --")
    for s in samples[:8]:
        print("  " + s[:200])
PY
)"

# ── Print formatted report ──
printf '%s\n' "$FMT"

# ── Persist a copy for cross-run comparison (gitignored) ──
HIST_DIR="$(cd "$(dirname "$0")" && pwd)/health-history"
mkdir -p "$HIST_DIR"
STAMP="$(date -u +%Y%m%d-%H%M%SZ)"
if [ "$JSON_MODE" = "1" ]; then
    EXT="json"
    OUT="health-$STAMP.summary.json"
else
    EXT="txt"
    OUT="health-$STAMP.${WINDOW}m.${EXT}"
fi
printf '%s\n' "$FMT" > "$HIST_DIR/$OUT"
ln -sfn "$OUT" "$HIST_DIR/latest.txt"
ln -sfn "$OUT" "$HIST_DIR/latest.$EXT" 2>/dev/null || true
echo "  (history: $HIST_DIR/$OUT)" >&2