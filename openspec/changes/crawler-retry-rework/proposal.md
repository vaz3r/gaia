## Why

Live failure data contradicts the "dead hashes never recover" assumption that drove `--max-attempts 2`: ~45% of torrents verified in the last 48h required a retry, and conversion rate *improves* with attempts (0.053% at attempt 1 → 0.109% at attempt 3). Meanwhile the 10% `unknown` and 4% `other` failure buckets are unexplained — they mix transient infrastructure failures (DHT lookup errors, pool exhaustion) with genuinely dead hashes, and are the most retry-productive classes. Retries today are also opportunistic: a failed hash is only re-fetched if the sampler re-stumbles on it, with no active drain of retry-eligible hashes.

## What Changes

- **A — Retry caps per failure class.** Raise the default retry budget from 2 to 4 attempts for transient classes (`timeout`, `deadline`, `unknown`, `connect_refused`), keep `empty_peers` (the 65% dead-hash churn, 0.08% conversion) capped at 2. `--max-attempts` becomes per-class aware.
- **B — Close the `unknown`/`other` classification gap.** Replace the three `dominant_failure: None` error sites (peer-hint dial, lookup-permit, get_peers) with real kinds (`DhtLookupFailed`, `LookupPoolExhausted`). Surface `other` fallback misses via debug logging so taxonomy gaps stay visible.
- **C — Retry schedule by class.** Transient classes get a shorter `next_attempt` backoff; dead-verdict classes keep the longer exponential backoff. Empty_peers loses its aggressive 60s re-fetch.
- **D — Parallel retry worker.** A new task actively polls `scanned` for `Failed WHERE next_attempt <= now AND attempts < cap(class)`, batches them, and emits `FetchRequest{source: Retried}` into the shared fetch queue with its own bounded semaphore (isolated from fresh-fetch slots). Adds `verified_retried` + `retry_worker_scans` counters and a `FetchSource::Retried` attribution path.
- **D-analysis — Grounded economics.** A reusable failures-analysis script quantifies per-class conversion so caps stay data-driven.

## Capabilities

### New Capabilities
- `retry-worker`: active, class-aware retry of failed hashes via a dedicated parallel worker feeding the fetch pipeline.
- `failure-classification`: complete failure taxonomy with no unexplained `unknown`/`other` sinks; per-class retry caps and schedules.

### Modified Capabilities
- `monitoring`: adds `verified_retried` and `retry_worker_scans` to the persisted snapshot and dashboard.
- `search`: no change.
- `storage/postgres`: adds a retry-eligible query over `scanned` (`next_attempt`/`attempts`/`failure_reason`).

## Impact

- **Code**: `crawler/src/cli.rs` (per-class caps), `crawler/src/fetch/failure.rs` (new kinds), `crawler/src/fetch/mod.rs` (classification + schedule), `crawler/src/discovery/sampler.rs` (class-aware terminal check), new `crawler/src/retry.rs` (worker), `crawler/src/discovery/mod.rs` (`FetchSource::Retried`), `crawler/src/stats.rs` + `crawler/src/crawler.rs` (counters + spawn), `crawler/src/storage/mod.rs` (retry query).
- **Ops**: higher fetch work on transient classes (bounded by per-class caps); `empty_peers` churn reduced.
- **Data**: `scanned.failure_reason` becomes fully classified going forward; existing rows stay as-is.
