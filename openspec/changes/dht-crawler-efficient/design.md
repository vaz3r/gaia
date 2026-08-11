## Context

Building on `dht-crawler-docker` (committed). The crawler runs on remote-dev behind Gluetun WireGuard with a public egress and healthy throughput (~200/hr), but bandwidth is high (~1.3 MB/s) and unique discovery is inefficient (~4% of sampled). This change re-targets the pipeline for torrents-per-byte. Adds decisions D28-D32.

## Goals / Non-Goals

**Goals:**
- Minimize bandwidth while maximizing unique infohash discovery and verified torrents.
- Stop re-sampling hashes already seen by any instance (shared dedup).
- Stop dialing peers that are known-dead to any instance (shared dead-peer cache).
- Let existing `get_peers` traffic double as routing-table discovery.
- Measure per-instance contribution so redundant instances are dropped.

**Non-Goals:**
- No content filtering (deferred by the user).
- No change to the public-IP/WireGuard architecture.
- No cross-host distributed crawling beyond local instances (4 by default).

## Decisions

### D28 — Cut the bandwidth bleed: lower budgets, fewer dials
`--qps` 8000 → 2000 and `--sampler-qps` 2000 → 400 per instance; `PARALLEL_DIALS` 32 → 8, `MAX_PEERS_PER_HASH` 100 → 25, `FETCH_DEADLINE` 20s → 10s; routing growers 100ms → 1s.
- *Rationale:* unique is ~4% of sampled — the sampler is re-querying the same BEP 51 nodes, not QPS-starved. The fetch pool dials ~16k connections against dead peers; 8 parallel × 25 peers is ample for the ~1-2% that verify. Growers were a startup-warmup hack; 1s steady-state suffices.
- *Trade-off:* slightly slower warmup and fewer dial chances per hash, but the pool was mostly idle and dead dials add nothing.

### D29 — Shared seen-set via Redis (optional, graceful)
Add a shared Redis `SEEN` set: when any instance emits a hash (passes min_seen), it `SADD`s the hash; other instances `SISMEMBER` and skip it. If Redis is unreachable, fall back to the in-memory per-instance `SeenCounts`.
- *Rationale:* the 96% duplicate problem is fleet-wide (instances overlap); a shared set stops re-emitting the same hashes across instances, freeing fetch slots for genuinely new hashes.
- *Alternatives considered:* per-instance only (current) — rejected, doesn't fix cross-instance duplication. A Redis bloom filter — rejected, SET is simpler at our scale.
- *Trade-off:* adds a Redis dependency; kept optional so the crawler degrades gracefully if Redis is down.

### D30 — Shared dead-peer cache via Redis
An IP that failed to connect ≥2 times in any instance is skipped by all instances for ~10 minutes. Redis-backed, optional/graceful like D29.
- *Rationale:* dead IPs are the same across instances; dialing them N times is pure waste.
- *Trade-off:* a recovered peer is skipped fleet-wide for up to the TTL.

### D31 — Discovery from `get_peers` + wider sampler spread
The DhtLookups already feed found nodes into the routing table for free (actor.rs:1126). Ensure `pick_target` samples across the full routing table (not the few nodes it last queried), so more distinct BEP 51 nodes are reached and more unique hashes surface per query.
- *Rationale:* distinct BEP 51 nodes drive unique discovery; random re-picking from a small ready set returns duplicates.
- *Trade-off:* none material — sampling the whole table is cheaper than re-querying a small hot set.

### D32 — Four instances with shared dedup; per-instance stats
Keep 4 instances (they multiply distinct BEP 51 node coverage), but the shared seen-set (D29) prevents them from re-fetching the same hashes — so 4 instances now add discovery breadth without the ~4x duplicate fetch waste they caused before the shared set existed. Add per-instance routing-node/sampled/unique stats so a contributor that only burns bandwidth is visible and can be dropped.
- *Rationale:* measuring showed 2 instances + min-seen 3 over-restricted the pool (200 → ~70/hr); the shared seen-set is what made multiple instances efficient, so 4 instances + min-seen 2 restores discovery while the low budgets (D28) and shared dedup keep bandwidth low.
- *Trade-off:* 4 instances cost more aggregate bandwidth than 2, but the shared set + low budgets keep it far below the original ~2 MB/s.

## Risks / Trade-offs

- **Redis down** → crawler degrades to per-instance behavior (still correct, just less efficient); guarded by error handling.
- **Lower budgets** → may briefly reduce absolute throughput; the goal is efficiency, and Phase D stats will confirm.
- **4 instances** → more aggregate discovery than 2; the shared seen-set prevents duplicate fetches, keeping bandwidth low.

## Migration Plan

No schema change. Deploy: rebuild image, `docker compose up -d --build` (4 instances + redis service). Rollback: keep the old compose/image. Redis is optional; if absent, crawler runs as before.
