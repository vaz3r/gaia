## Why

Measured on remote-dev, the passive-intake + node-diversity work reached ~108/hr (~2,600/day) — far below bitmagnet's documented 150-300k/day initial crawl. The bottleneck is structural, not tuning:

- Our routing table (`gaia-dht/routing_table.rs`) uses **K=8** nodes per bucket and only splits the last bucket → saturates at **~280 nodes/instance**.
- At ~280 nodes with a 60s per-node re-query cap, we sample only **~4.7 distinct BEP 51 nodes/sec/instance**.
- Bitmagnet's ktable uses **K=80** (`internal/protocol/dht/ktable/factory.go: nodesK = 80`) with a splittable trie → **thousands of nodes**, sampling **60 distinct nodes/sec** continuously.
- Our fetch path dials up to 16 peers even when `get_peers` returns no confirmed live values — bitmagnet bails immediately when `len(res.Values) < 1` (`get_peers.go:81`), so it never wastes dials on empty lookups.

The result: we discover ~1 unique hash/sec and verify ~1.3% of fetched; bitmagnet discovers ~100-200 unique/sec and verifies a far higher fraction because it only dials hashes with confirmed live peers. This change scales discovery to bitmagnet's architecture: a thousands-node routing table, a `--scale` concurrency knob mirroring bitmagnet's `scaling_factor`, higher sample throughput, and get_peers-first fetch selectivity.

## What Changes

- **Phase A — routing table to thousands**:
  - `K: 8 → 80` in `gaia-dht` (matches bitmagnet `nodesK=80`).
  - Verify bucket growth past ~280; if the last-bucket-only split policy still caps the table, extend splitting so full buckets split like bitmagnet's btree.
  - `closest(target, K)` responses grow to K nodes → lookups inject far more nodes per response.
- **Phase B — `--scale` knob (bitmagnet's `scaling_factor`)**:
  - New `--scale N` flag (default 10) multiplying sampler QPS, sampler loops, fetch concurrency, lookup concurrency, and pipeline buffer sizes.
- **Phase C — higher sample throughput**:
  - `sampler_qps` and `sampler_loops` scale with `--scale` so a thousands-node table is actually sampled at 50+ distinct nodes/sec.
- **Phase D — get_peers-first fetch selectivity**:
  - `fetch_one` only dials peers when `get_peers` returned confirmed live values (bitmagnet's `len(res.Values) < 1` bail); hinted (announce) peers are exempt (they're live by construction). Empty lookups fail fast as `empty_peers` instead of burning dials.

## Capabilities

### New Capabilities

- `routing-scale`: K=80 routing table holding thousands of nodes, more nodes injected per lookup.
- `scale-knob`: `--scale` concurrency multiplier (bitmagnet `scaling_factor` equivalent).
- `get-peers-selectivity`: only dial confirmed live peers; empty lookups fail fast.

### Modified Capabilities

- `dht` (previous changes): larger buckets; split policy verified/extended for table growth.
- `fetch` (previous changes): selectivity in `fetch_one`; higher concurrency via `--scale`.
- `cli` (previous changes): `--scale` flag; sampler budgets scale with it.
- `discovery` (previous changes): sampler QPS/loops scale for a larger node pool.

## Impact

- **Expected**: discovery 10-30x (thousands of nodes → ~50 distinct nodes/sec sampled → ~1k hashes/sec), and higher fetch success from get_peers-first selectivity. Combined, torrents/day should rise an order of magnitude or more, toward bitmagnet-scale.
- **Bandwidth**: more DHT queries (cheap UDP) and more fetches; still bounded by the shared dead-peer cache + 5s timeout and Oracle's 10 TB/mo free egress (current ~0.2 MB/s leaves huge headroom).
- **Risk**: K=80 increases per-response payloads and query volume; the `--scale` knob lets us tune. Table growth depends on the split policy — Phase A verifies and fixes it.
