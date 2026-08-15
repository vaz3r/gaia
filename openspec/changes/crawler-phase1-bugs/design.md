## Context

See `proposal.md` — Why. Seven verified bugs in the fetch/discovery pipeline. No schema or architecture change; all fixes are localized to the fetch path, redis dead-cache, stats loop, and dashboard rate.

## Goals / Non-Goals

**Goals:**
- Correct the lookup-permit lifetime so DHT concurrency is truly bounded.
- Accurate connect-failure classification + fleet-wide dead-peer marking.
- Reachable early-abort, full tracker-peer utilization, expiring Redis dead set.
- Correct dashboard rate and aggregate fleet metrics.

**Non-Goals:**
- No throughput tuning (SAMPLE_TIMEOUT, STALE_BACKOFF, parallel tracker+DHT, streaming dial pool) — those are a later sprint.
- No schema/migration.

## Decisions

### D1 — Hold the lookup permit across the stream (1.1)
Move `lookup_permits.acquire()` out of the `let mut peers = { ... }` block and keep the permit alive until the `'outer` recv loop breaks. Simplest: acquire before the block, hold a named `_lookup_permit` binding through the loop, drop after.
- *Rationale:* `get_peers_seeded` only enqueues a command; the lookup runs asynchronously and streams batches. Releasing at block end (current) lets all 1,536 fetch workers run lookups concurrently, defeating the 384 cap.
- *Trade-off:* holding the permit longer ties a fetch slot to the lookup duration; that is the intended behavior (RECV_TIMEOUT bounds the stream).

### D2 — Classify via `FetchFailureKind` and record dead for connect-level kinds (1.3+1.6)
In `dial_peers`, on `Ok(Err(e))`: compute `FetchFailureKind::from_error(&e)`. If the kind is `HandshakeFailed | NoUtMetadata | MetadataRejected | ParseError | Sha1Mismatch` → post-handshake (reset counter, set `any_handshake`). Otherwise (`Timeout`, `ConnectRefused`, `ConnectionReset`, `ConnectionClosed`, `Other`) → connect-level: increment `consecutive_connect_failures`, call `dead_peers.record_failure(ip)`, and `shared.dead_add(ip, 600)` on newly-dead.
- *Rationale:* the classifier already distinguishes these (failure.rs); use it as the single source of truth.

### D3 — `EARLY_ABORT_DIALS` 24 → 6 (1.2)
Constant change only.
- *Rationale:* `MAX_PEERS_PER_HASH=16` caps dials, so 24 consecutive failures was unreachable; 6 is a firm dead-hash signal within budget.

### D4 — Tracker peer batch loop (1.4)
Wrap the tracker `dial_peers` call in a loop over `tracker_peers` chunks of `PARALLEL_DIALS`, checking `deadline`/`MAX_PEERS_PER_HASH`/`tried` each iteration, returning on success.
- *Rationale:* reuses `dial_peers` unchanged; low-risk.

### D5 — Per-member expiry for `dht:dead` (1.5)
Replace the set+whole-key-EXPIRE with a **sorted set** `ZADD dht:dead <now> <ip>` and prune `ZREMRANGEBYSCORE dht:dead -inf <now-600>` before `dead_contains`/`dead_add`. `dead_contains` becomes `ZSCORE dht:dead <ip>` presence.
- *Rationale:* the whole-key EXPIRE resets on every insert (never expires under continuous crawling). A ZSET gives per-member time and O(log n) prune.
- *Trade-off:* `dead_contains` is now `ZSCORE` (similar cost to `SISMEMBER`).

### D6 — Dashboard rate from windowed history (1.7)
The dashboard's `live` card should use the API's rate endpoint (`/api/admin/monitor/rates?metric=metadata_verified&range=`) rather than dividing cumulative by snapshot age. Show the latest rate value.
- *Rationale:* the API already computes windowed rates via LAG(); the dashboard just consumes it. Correct and consistent with the monitoring charts.

### D7 — Aggregate all-instance metrics (4.1)
In `stats_loop`, sum `node_count()` and the DHT actor stats across all handles for `routing_nodes`, `active_lookups`, `announce_tokens`, `pending_queries`, `announces_*`, `lookups_received`, `announced_hashes`; keep `instance_nodes` per-instance.
- *Rationale:* currently these report instance 0 only (a documented observability gap); the fleet aggregate is what operators want.
- *Trade-off:* a handful more `.await` stat calls per 30s tick — negligible.

## Risks / Trade-offs

- **Holding the permit longer** could, under load, make fetches wait on the semaphore; that is the intended throttle. RECV_TIMEOUT bounds worst-case hold time.
- **Classifying `Other` as connect-level** might over-mark a rare post-handshake unknown as dead → it gets retried later (TTL 600s), acceptable.
- **Dead-set ZSET prune** adds a `ZREMRANGEBYSCORE` on the fetch hot path → run it opportunistically (every prune tick) not per-check.

## Migration Plan

1. Implement 1.1–1.6 (fetch + redis), test (unit + cargo test against Postgres).
2. Implement 4.1 (aggregate metrics), 1.7 (dashboard rate).
3. Deploy crawler + dashboard; verify aggregate stats, dead-set expiry, no regression in verified/hr over a clean window.
4. Rollback: revert the change; no data migration.

## Open Questions

None — the plan resolves each bug; verification is per-commit.
