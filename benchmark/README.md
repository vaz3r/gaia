# Benchmark scripts

Reusable `.sh` scripts to measure and compare crawler performance on remote-dev.
All default to the `./dht-crawler` compose stack (docker context `remote-dev`),
require the stack to be up, and accept a seconds window as `$1` and a compose
directory as `$2`.

| Script | What it measures | Example |
|---|---|---|
| `stats.sh` | Latest crawl stats + peer-failure breakdown from logs | `./benchmark/stats.sh 300` |
| `bandwidth.sh` | Tunnel bandwidth (MB/s) through Gluetun `tun0`, plus GB/day and GB/month projections vs Oracle Always Free limits | `./benchmark/bandwidth.sh 600` |
| `torrents_rate.sh` | Torrents found over a window → rate per hr / per day | `./benchmark/torrents_rate.sh 600` |
| `bench.sh` | Full report: stats + bandwidth + torrent rate + **efficiency (torrents/GB)** in one window | `./benchmark/bench.sh 600` |

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
- `torrents_rate.sh` and `bench.sh` count rows in the `torrents` table, which
  persists across restarts — rate is computed over the sampling window only.
- Windows under ~10 minutes are noisy (crawler traffic is bursty); prefer
  600s+.
