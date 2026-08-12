## Context

Building on `dht-crawler-passive-intake` (committed a3fac5a). Passive intake + node diversity reached ~108/hr but the routing table is the hard ceiling: K=8 buckets saturate at ~280 nodes, capping distinct-node sampling at ~4.7/sec/instance. Bitmagnet's K=80 splittable ktable reaches thousands of nodes and samples 60/sec. This change scales discovery and fetch selectivity to bitmagnet's architecture. Adds decisions D45-D48.

## Goals / Non-Goals

**Goals:**
- Grow the routing table from ~280 to thousands of nodes (K=80 + verified split policy).
- Add a `--scale` concurrency knob mirroring bitmagnet's `scaling_factor` (default 10, bumpable to 50).
- Raise sample throughput so a thousands-node table is actually sampled at 50+ distinct nodes/sec.
- Only dial confirmed live peers (get_peers-first), matching bitmagnet's `len(res.Values) < 1` bail.
- Raise torrents/day by an order of magnitude toward bitmagnet's initial-crawl scale.

**Non-Goals:**
- No content filtering.
- No change to instance count (4) or Docker/Gluetun/Redis architecture.
- No change to passive announce intake or the announce-first hint path (already built).

## Decisions

### D45 — K: 8 → 80 routing table
Raise `K` in `gaia-dht/src/routing_table.rs` from 8 to 80, matching bitmagnet's `nodesK=80`.
- *Rationale:* the table saturates around K × ~35 populated distance levels ≈ 280 nodes at K=8. At K=80 the same structure holds thousands, and `closest(target, K)` responses carry 80 nodes → lookups inject far more nodes per response.
- *Trade-off:* bigger per-response payloads and more nodes tracked per bucket; bounded by `max_nodes` (8192).

### D46 — Pre-allocated distance buckets + LRU eviction
Instead of the lazy last-bucket-only split policy (which lets far buckets fill to K and then permanently reject), the table pre-allocates all `MAX_BUCKETS` distance buckets keyed by exact leading-zeros distance; a full bucket evicts its least-recently-seen node.
- *Rationale:* bitmagnet's trie splits any full bucket; our old policy capped far buckets at K regardless of population, limiting the table to ~K × log2(N). Pre-allocated buckets let every distance level hold up to K, and LRU eviction keeps the freshest set.
- *Trade-off:* table shape changed (bucket_count is now always 160); verified by the growth test (>1000 nodes) and the full suite.

### D47 — `--scale` concurrency knob
New `--scale N` flag (default 10, like bitmagnet) multiplying sampler QPS, sampler loops, fetch concurrency, lookup concurrency, and pipeline buffer sizes.
- *Rationale:* bitmagnet users raise `scaling_factor` 10→50 for aggressive day-one aggregation; a single knob makes our concurrency tunable the same way.
- *Trade-off:* higher resource usage per unit; the default 10 matches bitmagnet's proven baseline.

### D48 — get_peers-first fetch selectivity
`fetch_one` SHALL only dial peers when `get_peers` returned confirmed live values; empty results fail fast as `empty_peers`. Hinted (announce) peers are exempt — they're live by construction.
- *Verified:* irontide's `DhtLookup` only emits non-empty peer batches (`dht_lookup.rs:413`), so the stock lookup already implements bitmagnet's `len(res.Values) < 1` bail — empty lookups close the stream immediately and `fetch_one` records `empty_peers` without dialing. No further code change needed for this decision.
- *Trade-off:* a hash whose get_peers returns no values is skipped this round (still retried via backoff); no live hashes are lost.

## Risks / Trade-offs

- **K=80 payload/query growth**: bounded by `--qps` and `--scale`; measured bandwidth stays the guardrail.
- **Split-policy change**: alters table dynamics; the existing 36 routing-table tests + a new growth test verify it.
- **Selectivity misses**: a hash with transiently-empty get_peers is deferred, not lost (backoff re-queues it).
- **Bandwidth creep**: more queries + more fetches; current ~0.2 MB/s vs Oracle's 10 TB/mo free egress leaves an order-of-magnitude of headroom.
