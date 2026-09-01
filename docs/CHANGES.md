# Changelog

## 2026-09-01 — Crawler Performance Tuning

**Date:** 2026-09-01
**Author:** opencode
**Files changed:** `deploy/targets/gaia-node/.env`, `apps/crawler/src/config.rs`

### Background

Health analysis of the crawler session (2026-09-01) revealed three performance issues:

1. **Low TCP/uTP connect success rate (3.3% TCP, 3.8% uTP)** — down from historical 4-5%
2. **Dead jobs accumulating at 4M+** — janitor can't keep up with the production rate
3. **Aggressive TCP timeout** — 3s default causing legitimate peers to fail

### Root Causes

- `find_node_response_percent=50` was set in `.env`, degrading DHT routing table quality and returning lower-quality peers
- Janitor defaults (25K batch every 30 min) are too slow for the current dead job production rate
- `tcp_timeout_secs=3` is too aggressive for public Internet peers, inflating `fetch_connect_timeout`

### Changes

| Change | File | Value | Rationale |
|--------|------|-------|-----------|
| `CRAW_FIND_NODE_RESPONSE_PERCENT` | `.env` | `50` → `100` | Restores full DHT routing table quality. Only 50% of `find_node` responses were being processed, degrading peer source quality and reducing connect success rate. |
| `CRAW_TCP_TIMEOUT_SECS` | `.env` | new, `5` | Increases TCP connect timeout from 3s to 5s. Many legitimate peers behind NAT or on congested links exceed 3s. This reduces `fetch_connect_timeout` and improves connect success rate. |
| `CRAW_JANITOR_INTERVAL` | `.env` | new, `300` | Janitor runs every 5 min instead of 30 min. Faster dead job cleanup prevents table bloat. |
| `CRAW_JANITOR_BATCH_SIZE` | `.env` | new, `100000` | 4x larger deletion batches (25K → 100K) to match dead job production rate. |
| `CRAW_JANITOR_BATCH_SLEEP_MS` | `.env` | new, `10` | Reduced sleep between batches (100ms → 10ms) for faster cleanup. |
| `CRAW_JANITOR_DEAD_RETENTION_SECS` | `.env` | new, `3600` | Dead jobs retained for 1 hour instead of 24 hours. Reduces table size by 24x. |
| `apply_env()` for janitor | `config.rs` | new | Adds env var support for `janitor_interval_secs`, `janitor_batch_size`, `janitor_batch_sleep_ms`, `janitor_dead_retention_secs` — previously only configurable via TOML. |

### Expected Impact

- **Connect rate:** Should restore to 4-5% (from 3.3%), boosting throughput 20-30%
- **Dead jobs:** Table should stabilize at ~100K-500K instead of growing to 4M+
- **Torrents/hr:** Should improve from 13k steady-state toward 16-20k
- **DB load:** Janitor will do more frequent but smaller deletions, smoothing I/O

### Risk Assessment

- **`find_node_response_percent=100`**: Low risk. This is the default in code and was only reduced to 50 in the `.env`. Restoring it to 100 means more `find_node` responses are processed, improving routing table quality. The only downside is slightly more CPU/routing work per lookup.
- **`tcp_timeout_secs=5`**: Low risk. Extends the window for slow peers. Combined with `utp_timeout_secs=5` (unchanged), both transports now have equal timeout budgets. The outer `connect_deadline_ms=15000` still caps the total race time.
- **Janitor tuning**: Low risk. More aggressive cleanup only affects stale dead job rows. Verified retention (3600s default) is unchanged.

### Rollback

All changes are in `.env` (except the `config.rs` env var support addition). To rollback:
1. Remove or revert the changed lines in `.env`
2. Redeploy the crawler container
