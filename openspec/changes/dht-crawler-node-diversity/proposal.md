## Why

The efficiency change (`dht-crawler-efficient`) cut bandwidth ~2.7x but **capped discovery**: measured on remote-dev the fleet samples 25.6M hashes yet finds only ~17k unique (0.067%), fetches 17k, verifies 190 (1.1% success). Steady-state output is now ~250-400 torrents/day, well below the earlier ~150/hr (~3,900/day) from the high-aggression phase.

Root cause: **the routing table is tiny (285 nodes across 4 instances)**. With a 60s per-node re-query cap, each instance can only sample ~4.75 distinct BEP 51 nodes/sec. The sampler keeps re-querying the same few nodes, which return the same ~17k hashes — most of them dead torrents. This is not a QPS problem; it is a **node-pool and hash-source diversity** problem.

Bitmagnet (pulled and analyzed) achieves far more because it:
- Continuously grows a large DHT ktable (find_node on the oldest nodes every second, pings every discovered node, feeds every response's `nodes` back in) — thousands of BEP 51 nodes.
- Samples **60 distinct ready nodes/sec** with per-node productivity deprioritization (0-new-hash nodes back off 5 min; productive nodes re-queried at a 60s cap).
- Rotates the `soughtNodeID` every 10s so find_node/sample targets spread across the keyspace.
- Uses a 10M-entry in-memory bloom filter so it never re-crawls a seen hash, and batches DB triage (~1000 hashes/20s) instead of per-hash lookups.
- Only requests metadata for hashes whose `get_peers` returned live peers.

This change adopts those mechanisms for our Rust/irontide stack: aggressively grow the node pool, sample more distinct nodes, add a second hash source (keyspace `get_peers` sweep + announce intake), and make dedup/triage cheap enough to keep up.

## What Changes

- **Phase A — node-pool growth (the core bitmagnet lever)**:
  - `grow_routing` interval 1s → 100ms per instance (4 lookups/s → ~40 lookups/s toward random targets), pushing the table from 285 toward `--max-nodes` (4096).
  - Verify response `nodes` feed-back and raise `PICK_CANDIDATES` spread now that the table is large.
  - Per-node **productivity deprioritization**: nodes returning 0 new hashes back off ~5 min instead of being re-sampled immediately.
- **Phase B — keyspace node growth (second node driver)**:
  - **Keyspace `get_peers` growth**: the faster growers (Phase A) now walk random 20-byte targets across the keyspace, growing the routing table into regions BEP 51 sampling under-weights — all with the stock irontide API.
  - ~~Announce intake~~ **measured and cut**: a `peer_store_hashes()` drain was implemented via a vendored irontide patch, but yielded only ~1.9% of unique hashes (103/5,415). The patch cost (vendored fork, upgrade risk) wasn't justified, so it was reverted; the `announced_hashes` counter stays diagnostic-only.
- **Phase C — cheap dedup / batch triage**:
  - 10M-entry in-memory bloom filter to short-circuit the per-hash DB `scan_blocked` on the sampler hot path.
  - Batch DB triage (~1000 hashes / 2s) instead of per-hash lookups.
- **Phase D — fetch pipeline for a larger stream**:
  - `FETCH_TIMEOUT` 10s → 5s for faster dead-peer churn; keep the `empty_peers` fast-path.
  - Confirm `concurrency=512` / `lookup_concurrency=256` keep up; add unique-hash **rate** to the stats line.

## Capabilities

### New Capabilities

- `node-diversity`: continuously growing routing table (100ms growers), wide `PICK_CANDIDATES` spread, productivity-based node deprioritization, keyspace `get_peers` node growth.

### Modified Capabilities

- `discovery` (previous changes): bloom-filter dedup on the sampler hot path, batch DB triage, unique-rate stats.
- `fetch` (previous changes): faster dead-peer timeout for a larger candidate stream.
- `cli` (previous changes): none new (announce-intake flags dropped with the reverted patch).

## Impact

- **Expected**: ~1,000-3,000 torrents/day at <1 MB/s (vs ~300/day now); keyspace crawling pushes toward bitmagnet-scale tens-of-thousands/day at higher bandwidth.
- **Bandwidth**: DHT query growth is cheap UDP; the binding constraint stays the TCP fetch churn, bounded by the shared dead-peer cache + 5s timeout.
- **State**: `peer_store` drains are best-effort (stateless snapshot); routing table persistence unchanged.
- **Risk**: more aggressive growers add DHT QPS; bounded by `--qps` and measured per-instance.

## Open Questions

- Exact grower cadence / DHT QPS after measuring Phase A's node-growth rate (currently 100ms/instance).
- Whether a future peer-store drain becomes worth a patched irontide once the announced-hash counter justifies it.
