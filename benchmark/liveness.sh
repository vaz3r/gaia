#!/usr/bin/env bash
# Crawler performance dashboard, read from the live SQLite DB on remote-dev.
#
# The crawler DB is a live WAL-mode SQLite file inside the `crawler` container
# on the `workspace-containers` host (docker context `remote-dev`). A plain
# `docker cp` of the DB file is torn (WAL mid-write => "malformed"), so we take
# a consistent snapshot as a single tar stream, then read it with host python3
# (no sqlite3 CLI is present in the container or on the host).
#
# Tables rendered as ASCII/box tables via python3 f-strings (no external libs).
#
# Usage:
#   benchmark/liveness.sh [hours] [--live]
#     hours  - window for the hourly tables (default 12)
#     --live - also pull the latest 30s stats line for liveness/shadow counters
#              (these live in the rotating log, not the DB; best-effort)
set -euo pipefail

HOURS=12
LIVE=0
for arg in "$@"; do
    case "$arg" in
        --live) LIVE=1 ;;
        [0-9]*) HOURS="$arg" ;;
        *) echo "unknown arg: $arg" >&2; exit 1 ;;
    esac
done

CONTAINER="crawler"
TMPDIR="${TMPDIR:-/tmp}/opencode"
DB_MAIN="$TMPDIR/crawler_perf.sqlite"
mkdir -p "$TMPDIR"

# 1. Consistent snapshot. The tar-stream of a live WAL DB races (main file
#    pages change while the WAL is being read => "malformed"), so use the
#    crawler's own `snapshot` command (VACUUM INTO, WAL-free, consistent).
rm -f "$TMPDIR/crawler_perf.sqlite"*
docker exec "$CONTAINER" crawler snapshot \
    --db /data/crawler.sqlite --out /data/crawler_perf.sqlite >/dev/null 2>&1 \
    || { echo "ERROR: could not snapshot $CONTAINER (is the stack up?)" >&2; exit 1; }
docker cp "$CONTAINER:/data/crawler_perf.sqlite" "$DB_MAIN" >/dev/null 2>&1 \
    || { echo "ERROR: could not fetch snapshot from $CONTAINER" >&2; exit 1; }
if [[ ! -s "$DB_MAIN" ]]; then
    echo "ERROR: snapshot is empty ($DB_MAIN)" >&2
    exit 1
fi

# 2. Live-gate status from the container args (best-effort).
ARGS="$(docker inspect "$CONTAINER" --format '{{.Args}}' 2>/dev/null | tr -d '[]' | tr ' ' '\n')"
MIN_SEEN="$(echo "$ARGS" | grep -A1 '^--min-seen$' | tail -1)"
MIN_SEEN_SHADOW="$(echo "$ARGS" | grep -A1 '^--min-seen-shadow$' | tail -1)"
[[ "$MIN_SEEN_SHADOW" =~ ^[0-9]+$ && "$MIN_SEEN_SHADOW" != "0" ]] || MIN_SEEN_SHADOW="off"

# 3. Query + render.
python3 - "$DB_MAIN" "$HOURS" "$LIVE" "$MIN_SEEN" "$MIN_SEEN_SHADOW" <<'EOF'
import sqlite3
import sys

db_path, hours, live, min_seen, min_seen_shadow = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], sys.argv[5]
db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)

def box(title, headers, rows, min_w=10):
    """Render one box-drawing table."""
    n = len(headers)
    col_w = []
    for i in range(n):
        w = max(min_w, len(headers[i]), *(len(str(r[i])) for r in rows)) if rows else max(min_w, len(headers[i]))
        col_w.append(min(w, 40))
    total = sum(col_w) + n + 1
    top = "┌" + "┬".join("─" * (w + 2) for w in col_w) + "┐"
    mid = "├" + "┼".join("─" * (w + 2) for w in col_w) + "┤"
    bot = "└" + "┴".join("─" * (w + 2) for w in col_w) + "┘"
    out = [f"  {title}", "  " + top]
    hdr = "  │ " + " │ ".join(h.ljust(col_w[i]) for i, h in enumerate(headers)) + " │"
    out.append(hdr)
    out.append("  " + mid)
    for r in rows:
        out.append("  │ " + " │ ".join(str(r[i]).ljust(col_w[i]) for i in range(n)) + " │")
    out.append("  " + bot)
    return "\n".join(out)

# --- Overall summary ---
total = db.execute("SELECT COUNT(*) FROM scanned").fetchone()[0]
ok = db.execute("SELECT COUNT(*) FROM scanned WHERE status='ok'").fetchone()[0]
failed = db.execute("SELECT COUNT(*) FROM scanned WHERE status='failed'").fetchone()[0]
torrents = db.execute("SELECT COUNT(*) FROM torrents").fetchone()[0]
success = 100.0 * ok / total if total else 0.0

print()
print(f"  CRAWLER PERFORMANCE   (crawler.sqlite @ remote-dev)   [--min-seen {min_seen}, --min-seen-shadow {min_seen_shadow}]")
print(box("OVERALL", ["metric", "value"], [
    ("total fetch attempts", f"{total:,}"),
    ("verified", f"{ok:,}"),
    ("failed", f"{failed:,}"),
    ("torrents indexed", f"{torrents:,}"),
    ("fetch success rate", f"{success:.2f}%"),
]))

# --- Verified torrents per hour (business metric) ---
rows = db.execute(f"""
    SELECT strftime('%Y-%m-%d %H', datetime(first_seen,'unixepoch')) AS hour,
           COUNT(*) AS n
    FROM torrents
    GROUP BY 1 ORDER BY 1 DESC LIMIT ?
""", (hours,)).fetchall()
print(box("VERIFIED PER HOUR (torrents)", ["hour", "count"], rows))

# --- Fetches vs verified per hour (efficiency over time) ---
rows = db.execute(f"""
    SELECT strftime('%Y-%m-%d %H', datetime(last_attempt,'unixepoch')) AS hour,
           COUNT(*) AS fetches,
           SUM(status='ok') AS verified
    FROM scanned
    GROUP BY 1 ORDER BY 1 DESC LIMIT ?
""", (hours,)).fetchall()
rows = [(h, f"{f:,}", str(v), f"{100.0*v/f:.2f}%" if f else "n/a") for h, f, v in rows]
print(box("FETCHES vs VERIFIED PER HOUR (scanned)", ["hour", "fetches", "verified", "success"], rows))

# --- Failure breakdown ---
rows = db.execute("""
    SELECT COALESCE(failure_reason,'none') AS reason, COUNT(*) AS cnt
    FROM scanned WHERE status='failed'
    GROUP BY 1 ORDER BY cnt DESC LIMIT 8
""").fetchall()
tot_failed = sum(c for _, c in rows)
rows = [(r if r else 'none', f"{c:,}", f"{100.0*c/tot_failed:.1f}%") for r, c in rows]
print(box("FAILURE BREAKDOWN", ["failure_reason", "count", "pct"], rows))

# --- Live liveness counters (rotating log, best-effort) ---
if live:
    import subprocess
    import re
    out = subprocess.run(
        ["docker", "logs", "crawler", "--since", "60s"],
        capture_output=True, text=True,
    ).stdout
    # Strip ANSI color codes: tracing emits key=<color>value<reset>, which
    # would split `liveness_entries=10559` in the middle.
    out = re.sub(r"\x1b\[[0-9;]*m", "", out)
    m = re.search(
        r"liveness_entries=(\d+).*?liveness_sweeps=(\d+)",
        out,
        re.S,
    )
    if m:
        rows = [
            ("liveness entries (dashmap)", f"{int(m.group(1)):,}"),
            ("liveness sweeps", m.group(2)),
        ]
        print(box("LIVENESS (from log, --live)", ["metric", "value"], rows))
    else:
        print("\n  (no recent crawl stats in log for --live; skipping liveness section)")

print()
EOF
