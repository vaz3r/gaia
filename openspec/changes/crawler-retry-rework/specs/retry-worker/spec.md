## Purpose

Retry failed torrent fetches actively and with per-failure-class policy, so transient infrastructure failures convert instead of being abandoned and dead-hash churn is bounded.

## ADDED Requirements

### Requirement: Class-aware retry budget
Each failure class SHALL have a maximum attempt count: transient classes (`timeout`, `deadline`, `unknown`, `connect_refused`) SHALL allow at least 4 attempts, and dead-verdict classes (`empty_peers`, `no_ut_metadata`, `metadata_rejected`, `sha1_mismatch`, `parse_error`) SHALL cap at 2.

#### Scenario: Transient failure gets more attempts
- **WHEN** a hash fails with `timeout` at attempt 1
- **THEN** it remains retry-eligible until its class cap (4) is reached

#### Scenario: Dead-verdict failure is terminal sooner
- **WHEN** a hash fails with `empty_peers` twice
- **THEN** it is no longer retried (terminal), bounding the dead-hash churn

### Requirement: Active retry worker
A dedicated worker SHALL periodically select retry-eligible failed hashes (`next_attempt <= now`, attempts below class cap) and re-fetch them, independent of whether the sampler re-reports them.

#### Scenario: Retry-eligible hash is re-fetched without re-sampling
- **WHEN** a failed hash's `next_attempt` has passed and it is under its class cap
- **THEN** the worker emits it for fetch without waiting for a sampler re-report

#### Scenario: Worker is concurrency-isolated
- **WHEN** the worker runs concurrently with fresh fetches
- **THEN** it uses its own bounded concurrency so it cannot starve the fresh-fetch path

### Requirement: Retried attribution
A retried fetch SHALL be attributed distinctly (a `Retried` source) so its conversion yield is measurable separately from fresh discovery sources.

#### Scenario: Retried verification is distinguishable
- **WHEN** a hash verifies via the retry worker
- **THEN** its verified count is attributed to the retried source and surfaced in monitoring

### Requirement: Complete failure classification
All fetch failures SHALL be assigned a concrete failure class; the `unknown`/`other` sinks SHALL be eliminated for the known error sites (DHT lookup, lookup-pool exhaustion, peer-hint dial), and unmatched fallback messages SHALL be logged for visibility.

#### Scenario: DHT lookup failure classified
- **WHEN** a `get_peers` lookup fails during a fetch
- **THEN** the failure is recorded as `dht_lookup_failed`, not `unknown`

#### Scenario: Unmatched fallback is visible
- **WHEN** a failure does not match any known classification
- **THEN** its message is logged (debug) so the taxonomy gap is discoverable

### Requirement: Retry schedule varies by class
Transient classes SHALL use a short backoff and dead-verdict classes the longer exponential backoff, so retries happen promptly where they convert and rarely where they do not.

#### Scenario: Transient class retries promptly
- **WHEN** a hash fails with `timeout`
- **THEN** its next attempt is scheduled after the short transient backoff

#### Scenario: Empty-peers no longer re-fetches every minute
- **WHEN** a hash fails with `empty_peers`
- **THEN** it is not re-fetched after the aggressive 60s window (its cap is 2)
