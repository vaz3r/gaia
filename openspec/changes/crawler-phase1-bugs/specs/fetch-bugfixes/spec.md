## Purpose

Correct fetch-pipeline bugs that waste work and misreport state: lookup concurrency accounting, connect-failure classification, dead-peer caching, tracker peer utilization, the early-abort threshold, Redis dead-set expiry, and the dashboard verified/hr rate.

## ADDED Requirements

### Requirement: Lookup concurrency is enforced
The fetch layer SHALL hold the lookup-pool permit for the entire DHT `get_peers` stream (not just the command send), so concurrent DHT lookups never exceed `lookup_concurrency`.

#### Scenario: Permit held across the stream
- **WHEN** a fetch performs a DHT lookup and consumes peer batches
- **THEN** the permit is released only after the stream ends or the deadline expires

#### Scenario: Concurrent lookups bounded
- **WHEN** many fetches run simultaneously
- **THEN** the number of active DHT lookups stays at or below the configured limit

### Requirement: Connect failures classified accurately
The fetch layer SHALL treat `ConnectionRefused`, `ConnectionReset`, and `BrokenPipe` as pre-connect failures (incrementing the consecutive-failure counter, not setting `any_handshake`), and SHALL treat only post-handshake failures (BEP 10 negotiation, metadata, SHA-1) as reachable-peer failures.

#### Scenario: Refused peer counts as a connect failure
- **WHEN** dialing a peer whose TCP connect is refused
- **THEN** the consecutive-connect-failure counter increments and the peer is marked dead, not treated as reachable

#### Scenario: Metadata failure resets the counter
- **WHEN** a peer completes the handshake but metadata fetch fails
- **THEN** the consecutive-connect-failure counter resets (the peer is reachable)

### Requirement: Early abort is reachable
The early-abort threshold SHALL be low enough to trigger within the per-hash peer budget, so a dead hash bails after a small number of consecutive connect failures.

#### Scenario: Dead hash aborts promptly
- **WHEN** a hash's dials fail to connect consecutively
- **THEN** the fetch aborts after the (reachable) early-abort threshold rather than burning the full dial budget

### Requirement: Tracker peers are fully utilized
The fetch layer SHALL iterate through all tracker-resolved peers in batches rather than discarding the remainder after the first batch fails.

#### Scenario: Remaining tracker peers tried
- **WHEN** the first batch of tracker peers fails
- **THEN** subsequent tracker peers are dialed until success, exhaustion, or deadline

### Requirement: Dead-peer set expires
The Redis dead-peer set SHALL expire individual entries so a continuously-crawling process does not accumulate dead IPs forever.

#### Scenario: Old dead entries expire
- **WHEN** a peer is marked dead and later a long period passes without re-marking it
- **THEN** its dead marking expires and the peer may be retried

### Requirement: Accurate verified rate
The dashboard verified/hr SHALL be computed from a proper time base (windowed history or process start), not from the 30-second snapshot timestamp.

#### Scenario: Rate is not inflated
- **WHEN** the dashboard displays verified/hr
- **THEN** the value reflects a meaningful window, not a division by the ~30s snapshot age

### Requirement: Aggregate fleet metrics
The stats loop SHALL report the sum across all instances for the core crawl metrics, while keeping the per-instance breakdown available.

#### Scenario: Routing nodes are summed
- **WHEN** the stats loop logs routing_nodes
- **THEN** it reflects the total across all instances, not only instance 0
