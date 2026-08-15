## Context

See `proposal.md` — Why. Live data shows retries convert (~45% of verified torrents in the last 48h required retries; conversion improves with attempts), the fetch pool is only ~22% utilized (avg 115/512 in-flight), and 14% of failures are unexplained (`unknown` 10% + `other` 4%). Retries today are opportunistic (only when the sampler re-reports a hash) and share the fetch pool.

## Goals / Non-Goals

**Goals:**
- Make retry conversion measurable and actively driven.
- Bound the 65% `empty_peers` dead-hash churn while keeping the productive transient retries.
- Eliminate unexplained `unknown`/`other` failure buckets at the known error sites.

**Non-Goals:**
- No change to discovery/sampling logic or keyspace coverage.
- No content filtering or classification of torrent *type* (unrelated to the "other classification" phrase in the request — that meant the `other` failure bucket).
- No change to the single-Postgres-store architecture.

## Decisions

### D1 — Per-class caps in one function, driven by `failure_reason`
A single `retry_cap(kind) -> u32` and `retry_delay(kind, attempts) -> i64` pair in `failure.rs` are the source of truth. The sampler's terminal-dead check, the worker's eligibility query, and the fetch failure path all call them.
- *Rationale:* one place to encode "transient = 4/short, dead = 2/long"; the DB already stores `failure_reason`, so both the sampler and worker can read class without new state.
- *Alternatives:* CLI-flag maps — rejected: adds config surface for constants best kept near the taxonomy.

### D2 — `FetchSource::Retried` attribution
Add a `Retried` source variant; verified counts split into `verified_retried`. `hashes_unique` is NOT incremented for retries (they're not new discoveries).
- *Rationale:* the dashboard and failure analysis need to see retry yield distinctly; reusing `Sampled` would conflate it with fresh discovery.
- *Trade-off:* one more source in the verified split; small.

### D3 — Retry worker shares the queue, owns its concurrency
`retry_worker` feeds the same `hash_tx` mpsc queue as the sampler (so `run_fetcher` unchanged) but holds its own `Semaphore` (default 64) so it can never starve fresh fetches even at pool peak.
- *Rationale:* the queue is the existing admission point (`scan_blocked_batch` runs there); a dedicated semaphore isolates retry load without a second fetch pipeline.
- *Alternatives:* separate fetch pool — rejected: duplicates `run_fetcher`/`in_flight`/dead-peer logic.

### D4 — Worker bypasses the sampler bloom
The sampler's `seen_bloom` caches terminal-dead verdicts; the worker checks `attempts < cap(class)` against the DB, not the bloom, so class-cap changes apply immediately and the worker never depends on sampler bookkeeping.
- *Rationale:* the bloom is a sampler-side short-circuit; the DB row is authoritative for retry eligibility.
- *Guard:* the worker shares the `in_flight` set with `run_fetcher` to avoid double-fetching.

### D5 — Worker cadence and batch
Poll every 30s (aligned with stats tick), select up to a bounded batch (e.g. 256) of `Failed WHERE next_attempt <= now AND attempts < cap` using the `scanned(last_attempt)` index added in M5, ordered by `next_attempt`.
- *Rationale:* 30s aligns with monitoring; the batch bound + semaphore cap worker throughput to a controlled fraction of the idle pool.

### D6 — Class schedule
`retry_delay`: transient (`timeout`, `deadline`, `unknown`, `connect_refused`, `dht_lookup_failed`, `lookup_pool_exhausted`) → `min(60*2^(attempts-1), 10m)`; dead-verdict (`empty_peers`, `no_ut_metadata`, `metadata_rejected`, `sha1_mismatch`, `parse_error`) → existing `backoff_secs` (1m..6h), and `empty_peers` loses its special 60s fast path.
- *Rationale:* the data shows transient classes convert better on retry; empty_peers is 0.08% and dominates volume — capping it at 2 and dropping the 60s re-fetch removes the bulk of the churn.

## Risks / Trade-offs

- **More fetch work on transient classes** → bounded by class caps (≤4) and the worker's own 64-slot semaphore; pool is 22% idle, so headroom is ample.
- **Retry worker could double-fetch a hash** → guarded by the shared `in_flight` set + `scan_blocked_batch` at the queue admission point.
- **New failure kinds change existing `failure_reason` strings** (`dht_lookup_failed`, `lookup_pool_exhausted`) → the dashboard/failure analysis treat them as new buckets; existing rows keep their old values.
- **empty_peers cap 2 drops some conversions** (949 all-time from retries) → net win: those 949 cost millions of failed re-fetches; the worker + transient-class retries recover more than the empty_peers tail, and A/B verifies it.

## Migration Plan

1. Implement A (caps) + B (classification) first — independently testable; `unknown` bucket should collapse to near-zero.
2. Implement C (schedules) — the empty_peers 60s removal.
3. Implement D (worker) — spawn in `crawler::run`, add counters + `FetchSource::Retried`.
4. Deploy; A/B `--max-attempts 4` + worker on vs off over a clean window using `benchmark/failures_analysis.sh` + `bench.sh`.
5. Rollback: remove the worker spawn + revert caps via git; no data migration needed (old `failure_reason` rows remain valid).

## Open Questions

None — decisions above resolve the retry policy; remaining tuning (exact batch size, worker cadence) is settled by A/B after deploy.
