# Benchmark scripts

Reusable `.sh` scripts to measure and compare crawler performance on remote-dev.
All default to the `./crawler` compose stack (docker context `remote-dev`),
require the stack to be up, and accept a seconds window as `$1` and a compose
directory as `$2`.

| Script | What it measures | Example |
|---|---|---|
| `bench.sh` | Full windowed report: stats + bandwidth + torrent rate + **efficiency (torrents/GB)** in one window | `./benchmark/bench.sh 600` |
| `liveness.sh` | **SQLite performance dashboard**: overall fetch/verified/success, verified per hour, fetches vs verified per hour, failure breakdown, and (with `--live`) the liveness-gate counters | `./benchmark/liveness.sh 12` / `./benchmark/liveness.sh 6 --live` |

## Archived experiments (`experiments/`)

`experiments/` holds the artifacts of one-off experiments, kept for
reproducibility of the findings cited in `VALIDATION.md`. They are not part of
the regular benchmark toolset.

- `instances-ab.sh` + `docker-compose.1inst.yml` — the 8-vs-1 instance A/B test
  behind the **"single-IP ceiling accepted"** closing decision
  (`VALIDATION.md`). See `experiments/docker-compose.1inst.yml` for the
  protocol and confound notes.

## The performance dashboard (`liveness.sh`)

Reads the live `scanned` + `torrents` tables on remote-dev and renders ASCII/box
tables — the rotation-proof source of truth (no reliance on rotating logs). The
live WAL DB is snapshotted via the crawler's `snapshot` command (VACUUM INTO),
then read with host `python3` (no sqlite3 dependency).

| Section | What it answers |
|---|---|
| Overall | total fetch attempts, verified, failed, torrents indexed, fetch success rate |
| Verified per hour | the business metric (new torrents indexed per hour) |
| Fetches vs verified per hour | efficiency over time (attempts → verified, success %) |
| Failure breakdown | why bandwidth is spent (`empty_peers`/`timeout`/etc.) |
| `--live` liveness | `liveness_entries` (dashmap size) + `liveness_sweeps` (best-effort, from the rotating log) |

## Reading efficiency

`bench.sh` reports torrents/GB; the original 4-instance config measured ~27.8k
torrents/GB. The efficiency phase improved this to ~55k torrents/GB at lower
bandwidth. Use `bench.sh` after any crawler config change to compare.

## Bandwidth reference (Oracle Always Free)

- Outbound (egress/upload): 10 TB/month free — crawler uses a few percent.
- Inbound (ingress/download): free/unmetered.

## Notes

- DB snapshots are read-only copies via `docker cp`; the live DB is never
  touched.
- `torrents` table counts persist across restarts — rate is computed over the
  sampling window only.
- Windows under ~10 minutes are noisy (crawler traffic is bursty); prefer
  600s+.
