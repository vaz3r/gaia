## 1. Classification gap (B)

- [x] 1.1 Add `DhtLookupFailed` and `LookupPoolExhausted` variants to `FetchFailureKind` with `as_str` forms `dht_lookup_failed` / `lookup_pool_exhausted`
- [x] 1.2 Classify the three `dominant_failure: None` error sites in `fetch/mod.rs`: lookup-permit acquire → `lookup_pool_exhausted`; `get_peers` error → `dht_lookup_failed`; peer-hint dial → classify via `FetchFailureKind::from_error` instead of `None`
- [x] 1.3 Add a debug log for unmatched `from_string` fallbacks in `failure.rs` (raw message, marker `unmatched_failure`)
- [x] 1.4 Unit tests: new kinds round-trip via `as_str`/`from_string`; the three error sites produce the new kinds

## 2. Class-aware caps + schedules (A, C)

- [x] 2.1 Add `retry_cap(kind: Option<&str>) -> u32` (transient ≥4, dead-verdict 2) and `retry_delay(kind, attempts) -> i64` (transient short, dead-verdict long) in `failure.rs`; tests for both
- [x] 2.2 Update the sampler's terminal-dead check (`sampler.rs:515`) to use `retry_cap(failure_reason)` instead of the flat `max_attempts`
- [x] 2.3 Update the fetch failure path (`fetch/mod.rs:249-253`) to use `retry_delay(kind, attempts)`; remove the `empty_peers` 60s fast path
- [x] 2.4 Change `--max-attempts` default to 4 (documented as the transient-class budget); keep the flag for explicit override

## 3. Retry worker (D)

- [x] 3.1 Add `retry_eligible()` to `Storage`: `SELECT info_hash, failure_reason, attempts FROM scanned WHERE status='failed' AND next_attempt <= $now AND attempts < $cap ORDER BY next_attempt LIMIT $batch` (uses the existing `last_attempt` index)
- [x] 3.2 Create `retry.rs` with `run_retry_worker(storage, hash_tx, semaphore, stats, shutdown)`: poll every 30s, batch ≤256, emit `FetchRequest{source: Retried}`; skip hashes in the shared `in_flight` set
- [x] 3.3 Add `FetchSource::Retried`; route `verified_retried` in `persist_verified`/`record_verified`; do NOT bump `hashes_unique` for retries
- [x] 3.4 Add `retry_worker_scans` counter; wire both into `CrawlStats`, the stats log, and `CrawlSnapshot`/`record_crawl_stats`
- [x] 3.5 Spawn `run_retry_worker` in `crawler::run` with a 64-slot semaphore; share `in_flight` with the fetcher (refactor `in_flight` to an `Arc<Mutex<HashSet>>` passed to both)
- [x] 3.6 Unit test the worker's eligibility + emit logic against a temp Postgres (no in_flight collision, respects cap)

## 4. Analysis + integration

- [x] 4.1 Add `benchmark/failures_analysis.sh` (or .md doc) with the per-class conversion SQL from the investigation
- [x] 4.2 Add `verified_retried` + `retry_worker_scans` to the dashboard monitoring snapshot surface (admin API + dashboard display)
- [x] 4.3 Full `cargo test` green against Postgres; clippy clean; release build
- [x] 4.4 Deploy; verify `unknown` bucket collapses, `verified_retried` grows, pool stays <60% utilized
- [x] 4.5 A/B clean window: `--max-attempts 4` + worker on vs off via `bench.sh`; record verified/hr + conversions
