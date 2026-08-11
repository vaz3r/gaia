## Why

The Docker + Gluetun deployment (`dht-crawler-docker`) proved the public egress raises the verify rate, but it revealed an efficiency problem: **high bandwidth for modest discovery**. Measured on remote-dev:

- ~200 torrents/hr at **~1.3 MB/s sustained** (~4.7 GB in ~50 min) through the tunnel.
- Unique discovery ~14/s, but **only ~4% of sampled hashes are unique** (587k sampled → 25k unique). The sampler re-queries the same ~20-30 BEP 51 nodes, which return the same ~400-600 hashes.
- Fetch pool dials up to 32 peers in parallel, 100 peers/hash, 20s deadline — **up to ~16k concurrent TCP connections**, mostly to dead peers. This TCP churn is the bandwidth hog and adds zero discovery.
- 4 instances mostly overlap (same routing-table neighborhood, same BEP 51 nodes): ~4x bandwidth for ~1.1x unique.

The stated goal is **low bandwidth, high discovery rate**. This change re-targets every subsystem to maximize torrents discovered per byte: stop re-sampling what we've seen, stop dialing dead peers, share dedup state across instances, and let existing `get_peers` lookups double as node discovery.

## What Changes

- **Phase A — cut the bleed**:
  - `--qps` 8000 → 2000, `--sampler-qps` 2000 → 400 per instance (the sampler isn't QPS-starved; it's re-querying the same nodes).
  - Fetch churn: `PARALLEL_DIALS` 32 → 8, `MAX_PEERS_PER_HASH` 100 → 25, `FETCH_DEADLINE` 20s → 10s.
  - Routing growers throttled 100ms → 1s.
- **Phase B — raise discovery per byte**:
  - **Redis shared seen-set**: one shared `SEEN` set so instances stop emitting hashes another already found (attacks the 96%-duplicate problem fleet-wide). Redis already runs on this host; the crawler container connects to a Redis service in the stack. Optional/graceful — if Redis is unreachable, fall back to the in-memory per-instance seen map.
  - **Redis shared dead-peer cache**: skip an IP that failed to connect fleet-wide, cutting duplicate dial churn.
  - **`pick_target` spread**: sample across the full routing table rather than the few ready nodes the sampler already knows.
  - **`get_peers` as discovery**: the DhtLookups already feed found nodes into the routing table (actor.rs:1126); with tighter fetch budgets this now happens more cheaply, and peers returned are added to the shared seen/dial pool.
- **Phase C — verify quality**: keep `--min-seen 2`; 4 instances (shared seen-set prevents duplicate fetches, so more instances add discovery without the fetch waste).
- **Phase D — measure**: per-instance stats (routing nodes, sampled/unique rates) so an instance that only burns bandwidth can be detected and dropped.

## Capabilities

### New Capabilities

- `efficient-discovery`: shared seen-set and dead-peer cache via Redis, full-table node spread, and get_peers-as-discovery — maximizing unique hashes and verifications per unit of bandwidth.

### Modified Capabilities

- `discovery` (previous changes): shared dedup, wider node spread, throttled growers.
- `fetch` (previous changes): far lower dial churn, shared dead-peer cache.
- `architecture` (previous changes): 4 instances + optional Redis service in the stack.
- `cli` (previous changes): lower QPS/sampler defaults, `--min-seen 3`, `--redis-url`.

## Impact

- **Code**: `cli.rs` defaults/flags; `fetch/mod.rs` constants + Redis-backed dead-peer; `discovery/sampler.rs` shared seen + pick spread; `crawler.rs` grower throttle + per-instance stats; new `redis.rs` module.
- **Dependencies**: add `redis` (Rust client) — optional at runtime; `docker-compose.yml` gains a `redis` service.
- **Operations**: bandwidth expected to drop ~3-6x (~1.3 → ~0.2-0.4 MB/s); unique discovery up ~2-3x; torrents/hr toward ~300-500. Oracle free-tier bandwidth limit respected.
- **Performance (expected)**: torrents per byte up several-fold; verify rate up via min-seen 3.
