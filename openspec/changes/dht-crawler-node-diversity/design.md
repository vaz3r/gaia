## Context

Building on `dht-crawler-efficient` (committed, D28-D32). The crawler is efficient on bandwidth (~0.3-0.75 MB/s) but discovery has plateaued: 285 routing nodes, ~0.067% unique sample rate, ~250-400 torrents/day. Analysis of bitmagnet's crawler (`internal/dhtcrawler`, `protocol/dht/ktable`, `metainforequester`) shows the winning mechanisms are node-pool growth, distinct-node sampling, cheap dedup, and more hash sources. Adds decisions D33-D38.

## Goals / Non-Goals

**Goals:**
- Grow the routing table from ~285 to thousands of nodes (bitmagnet-scale BEP 51 coverage).
- Sample more distinct nodes/sec via productivity-aware selection.
- Add a second infohash source: keyspace `get_peers` sweep + DHT announce intake.
- Make dedup/triage cheap so the pipeline keeps up without per-hash DB hits.
- Raise verified torrents/day to ~1,000-3,000 at <1 MB/s, and toward bitmagnet-scale with keyspace crawling.

**Non-Goals:**
- No content filtering (deferred).
- No change to Docker/Gluetun architecture or Redis coordination model.
- No replacement of irontide; changes are additive (handle methods, sampler/fetch tuning).

## Decisions

### D33 — 100ms routing growers (node-pool growth is the bottleneck)
Run `grow_routing` every 100ms per instance instead of 1s, issuing `get_peers` lookups toward random 20-byte targets so the routing table climbs toward `--max-nodes` (4096) throughout the crawl.
- *Rationale:* 285 nodes → 4.75 sampleable nodes/sec/instance caps discovery regardless of QPS budget. bitmagnet grows its ktable with find_node on old nodes every second; our cheapest equivalent is faster get_peers growers (they already inject nodes). ~40 lookups/s across 4 instances is a small UDP cost vs the discovery ceiling it removes.
- *Trade-off:* more DHT QPS; bounded by `--qps` (2000). Risk of lower-quality nodes; mitigated by existing `checked_insert` quality controls and per-node productivity stats.

### D34 — Productivity-based node deprioritization
Track per-node new-hash yield; a node returning 0 new unique hashes backs off ~5 min (instead of being re-queryable at its advertised interval). Productive nodes keep a short re-query cap.
- *Rationale:* mirrors bitmagnet's `NodeSampleInfoHashesRes` deprioritization; stops re-sampling dead BEP 51 nodes that return the same old hashes.
- *Trade-off:* a temporarily quiet node is skipped for up to 5 min.

### D35 — Keyspace `get_peers` node growth via the growers
The faster growers (D33) issue `get_peers` lookups toward random 20-byte targets across the ID space, growing the routing table in keyspace regions the sampler's own queries don't reach. Uses only the stock irontide API.
- *Rationale:* BEP 51 sampling tops out; random-target lookups add node diversity in parallel, and the growers already do this — no separate sweep loop needed.
- *Trade-off:* additional DHT QPS; bounded by `--qps` and the shared dead-peer cache keeps fetch churn low.

### D36 — Announce intake from irontide's peer_store (REVERTED after measurement)
Originally proposed adding a `peer_store_hashes()` handle method (new `DhtCommand::PeerStoreHashes` reading the actor's peer store) and feeding announced infohashes into the pipeline.
- *Outcome:* implemented via a vendored `[patch.crates-io]` irontide-dht copy, measured on remote-dev, then **reverted**. The announced-hash drain contributed only ~1.9% of unique hashes (103 of 5,415) at the cost of vendoring/forking irontide — the one non-additive change in the plan, with recurring upgrade/maintenance risk. Not worth it.
- *Decision:* keep announce volume as a diagnostic counter only (`announced_hashes`); revisit only if a future need outweighs the patch cost.

### D37 — Bloom-filter dedup + batch DB triage
Sampler uses a ~10M-entry in-memory bloom filter to short-circuit the per-hash `scan_blocked` DB read on the hot path; DB triage is batched (~64-hash chunks per refill) so pipeline admission is cheap and burst-tolerant.
- *Rationale:* bitmagnet's `ignoreHashes` bloom filter + batched triage is exactly this; removes the per-hash SQLite query that becomes a bottleneck when unique discovery rises.
- *Trade-off:* bloom false-positive risk (~0.1% at our sizing) skips a rare new hash until it is re-sighted; acceptable. Backoff-state hashes are intentionally NOT cached so they can be retried after expiry.

### D38 — Fetch tuning for a larger stream
`FETCH_TIMEOUT` 10s → 5s to churn dead peers faster; keep `empty_peers` fast-path and shared dead-peer cache. Add unique-hash **rate** (unique/hr) and per-source counters to the stats line.
- *Rationale:* as more distinct hashes flow, per-hash wall-clock dominates; 5s still catches the ~1-2% that verify while freeing slots.
- *Trade-off:* a slow-but-alive peer might miss a 5s window; acceptable given the shared cache re-dials it later.

## Risks / Trade-offs

- **irontide stays stock (D36)**: announce intake was measured and cut rather than patching irontide; the peer-store counter remains diagnostic-only.
- **Bandwidth creep**: more growers add UDP QPS; fetch churn stays the binding cost and is bounded by dead-peer cache + 5s timeout. Phase D stats will show MB/s alongside rates.
- **Bloom false positives**: bounded at ~0.1%; rare new hashes may be delayed one batch, not lost permanently.
