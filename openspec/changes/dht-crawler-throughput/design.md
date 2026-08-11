## Context

Building on `dht-crawler-optimization` (committed). The fetch pool is unblocked but underutilized, verify rate is 0.58%, dead hashes burn full deadlines, backoff is too slow for the long tail, and discovery is single-node. This change adds decisions D13–D17 to the existing D1–D12 foundation.

## Goals / Non-Goals

**Goals:**
- Push fetch-pool utilization from ~17% toward saturation by removing the lookup gating and instrumenting the actual limiter.
- Raise verify rate by (a) keeping multi-confirmed hashes (`min-seen 2`), (b) not re-dialing dead peers, and (c) failing dead hashes early.
- Multiply discovery breadth with N DHT instances sharing one DB.
- Retry dead hashes fast enough to catch swarms that appear quickly.
- A solid, reviewable openspec with per-tier scenarios.

**Non-Goals:**
- No public tracker/index sources.
- No vendoring/patching irontide (announced_hashes confirms it's not worth it).
- No distributed crawling across machines — only multiple local instances.
- No sharding the SQLite writer across threads; one writer, N samplers/fetchers.

## Decisions

### D13 — Unblock the pipeline: raise lookup budget and instrument
Raise `--lookup-concurrency` default 64→256 and `--qps` 5000→8000 (aggressive 256→512 lookups, qps 12000). Add `fetch_in_flight` and `queue_depth` counters to the stats output so the true limiter is measurable before/after.
- *Rationale:* the pool spawns 4.3/s vs ~25/s capacity; the suspects are lookup-permit gating and the shared actor query budget. Raising both plus instrumentation lets us confirm.
- *Alternatives considered:* splitting the DHT budget between sampler and lookups — rejected as a premature coupling; measure first.

### D14 — Fail dead hashes fast
`FETCH_DEADLINE` 20s→12s. Additionally, track the first batch's dial outcomes per hash; if every dial in the initial window (first ~24 peers) ends in connect timeout/refused with no successful handshake, abort immediately rather than waiting out the deadline.
- *Rationale:* 21% of failures are "deadline" (full 20s burned); the first few dials predict the rest almost always.
- *Trade-off:* a transiently slow-but-live peer could be missed; acceptable — such peers rarely serve metadata within a deadline anyway.

### D15 — In-run dead-peer cache
Maintain an in-memory map `IpAddr → last-failure unix time`, TTL ~10 minutes. When dialing, skip any peer whose IP failed ≥2 connects in the TTL window (failures counted across all hashes, not just this one). Clear entries lazily on TTL expiry.
- *Rationale:* 9,148 connect timeouts across 2,595 fetches = the same dead IPs re-dialed for every hash.
- *Trade-off:* a recovered peer is skipped for up to 10 min; acceptable and standard crawler practice.

### D16 — Multi-instance crawling
Add `--instances N` (default 1). For each instance: bind UDP on `port + i`, use `state-dir/instance-i/`, and spawn its own sampler. All instances share one `Storage` handle and one fetch pool. `get_peers`/sample traffic scales with N; the DB writer stays single.
- *Rationale:* `announced_hashes≈0` proves passive discovery is dead on NAT; active sampling breadth is the only lever, and N independent node IDs × routing tables multiply unique hashes.
- *Alternatives considered:* threads sharing one node ID — rejected, routing tables would merge into one discovery surface. Separate state dirs keep N independent keyspaces.
- *Trade-off:* N× the UDP/query load; `--aggressive` should be used with N≥2 only on a VPS.

### D17 — Routing warmup
At startup, before the sampler ramps, issue `get_peers` on ~16 random targets (throttled) to force `find_node`/`get_peers` cascades that populate the routing table faster than passive sampling alone. Then hand off to the normal sampler.
- *Rationale:* the routing table reached only 212/2048 nodes in 10 min; more nodes → more BEP 51-capable nodes → more hashes.
- *Alternatives considered:* more sampler loops instead — kept but warmup is cheap and targeted.

### D18 — Retry policy
Backoff base 5m→60s (exponential, cap 6h). Give `empty_peers` failures a dedicated short window: retry after 60s (they may gain peers within a minute), while `timeout`/`deadline`/`other` keep the standard exponential backoff.
- *Rationale:* a hash with zero peers now often gains peers within a minute; 5m backoff wasted that.
- *Trade-off:* slightly more re-fetch churn; bounded by the pool's failure path being cheap (early abort).

## Risks / Trade-offs

- **Aggressive budgets may look noisy on shared links** → document that `--instances`, `--lookup-concurrency`, `--qps` are tunable down.
- **Multi-instance DB contention** → one writer via `Storage`, WAL mode; reads via the reader connection are safe.
- **Dead-peer TTL may skip a recovered peer briefly** → 10 min, configurable constant.
- **Early abort may miss slow-but-live peers** → rare for metadata serving; acceptable.

## Migration Plan

No schema change in this change (backoff is computed in code, not stored). Existing databases are untouched. Multi-instance adds state dirs under `state-dir/instance-N/` (new, no migration). Rollback: revert to a single instance (`--instances 1`); no external systems touched.
