## Purpose

Complete, observable failure classification with per-class retry policy, so every fetch failure carries an actionable label and no unexplained sink hides recoverable torrents.

## ADDED Requirements

### Requirement: No unexplained failure buckets
The fetch layer SHALL assign a concrete failure class to every failure from the known error sites (peer-hint dial, lookup-pool acquisition, DHT get_peers), eliminating `unknown`/`other` as the recorded reason for those sites.

#### Scenario: Lookup-pool exhaustion classified
- **WHEN** a fetch cannot acquire a lookup permit
- **THEN** the failure is recorded as `lookup_pool_exhausted`

#### Scenario: Peer-hint dial classified
- **WHEN** a fetch's direct peer-hint dial fails
- **THEN** the failure is recorded under its actual underlying cause, not `unknown`

### Requirement: Unmatched classifications are visible
When a failure does not map to a known class, the fetch layer SHALL log the raw message at debug level so new taxonomy gaps are discoverable rather than silently folded into `other`.

#### Scenario: Fallback gap logged
- **WHEN** a failure message matches no known classification
- **THEN** the message is logged with a debug marker indicating an unmatched fallback

### Requirement: Class-aware retry caps and schedules
Retry budget and backoff SHALL depend on the failure class: transient classes permit more attempts and a short backoff; dead-verdict classes permit few attempts and the longer exponential backoff.

#### Scenario: Caps and schedules follow class
- **WHEN** the sampler or retry worker decides whether a failed hash may be retried
- **THEN** the decision uses the hash's `failure_reason` to pick the class cap and schedule
