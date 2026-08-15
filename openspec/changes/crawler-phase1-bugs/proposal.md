## Why

An audit (`PLAN.md`) found seven real bugs in the fetch/discovery pipeline that waste fetch work and misreport state. All were verified against the code: the lookup-permit semaphore is released before the DHT lookup actually runs (bypassing the 384-limit → KRPC flood), connection-refused is misclassified as a successful handshake, the early-abort path is unreachable, tracker peers beyond the first 4 are discarded, the Redis dead set never expires, and the dashboard shows an inflated verified/hr. Fixing these is a correctness win and removes wasted work before any throughput tuning.

## What Changes

- **1.1 — Hold the lookup permit across the whole `get_peers` streaming loop**, not just the command send, so the `lookup_concurrency` semaphore actually bounds concurrent DHT lookups.
- **1.2 — Lower `EARLY_ABORT_DIALS` 24 → 6** so the early-abort path is reachable (it was dead code because `MAX_PEERS_PER_HASH=16` capped the counter below 24).
- **1.3+1.6 — Classify pre-connect failures correctly**: `ConnectionRefused`/`ConnectionReset`/`BrokenPipe` are connect failures (increment the consecutive-failure counter, do NOT set `any_handshake`), and they mark the peer dead in the in-process + Redis dead-peer cache. Only post-handshake failures (BEP 10, metadata, SHA-1) reset the counter.
- **1.4 — Loop over all tracker peers in batches of `PARALLEL_DIALS`** instead of discarding the tail after the first failed batch.
- **1.5 — Fix the Redis `dht:dead` set TTL**: per-member timestamps (or per-IP TTL keys) so the set actually expires dead entries instead of resetting the whole-key TTL on every insert.
- **1.7 — Fix the dashboard verified/hr rate** to use a windowed/process-start denominator instead of the 30s snapshot timestamp (which inflates it ~100x).
- **4.1 — Aggregate all-instance metrics** (`routing_nodes`, `active_lookups`, `announce_tokens`, `pending_queries`, `announces_*`, `lookups_received`) so the stats loop reports the fleet-wide sum, keeping the per-instance breakdown in `instance_nodes`.

## Capabilities

### New Capabilities
- `fetch-bugfixes`: correct lookup concurrency accounting, accurate connect-failure classification + dead-peer caching, tracker peer utilization, and a reachable early-abort.

### Modified Capabilities
- `monitoring`: aggregate (all-instance) crawler metrics and an accurate verified/hr rate.
- `admin-api`: no API change; the dashboard consumes existing history for the corrected rate.

## Impact

- **Code**: `crawler/src/fetch/mod.rs` (permit scope, EARLY_ABORT, classifier, tracker loop, dead-peer), `crawler/src/fetch/wire.rs` (classifier contract), `crawler/src/redis.rs` (dead-set TTL), `crawler/src/crawler.rs` (aggregate metrics), `dashboard/src/features/monitoring/MonitoringPage.tsx` (rate).
- **Behavior**: correct failure taxonomy (dead hashes bailed sooner, dead IPs skipped fleet-wide), accurate concurrency bounds, no inflated dashboard rate.
- **No schema/data migration.**
