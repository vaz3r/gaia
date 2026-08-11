## Purpose

Cut metadata-fetch churn to a minimum while keeping the pool productive: far fewer peer dials per hash, a short deadline, and a shared fleet-wide dead-peer cache.

## ADDED Requirements

### Requirement: Low per-hash dial budget
The fetch layer SHALL dial at most 8 peers concurrently, try at most 25 distinct peers per hash, and give each hash at most 10 seconds.

#### Scenario: Dead hash frees its slot fast
- **WHEN** a hash's peers are unreachable
- **THEN** the fetch ends after at most 10s / 25 peers / 8 parallel dials

#### Scenario: Successful fetch still completes
- **WHEN** an early dial serves matching metadata
- **THEN** the fetch succeeds without exhausting the budget

### Requirement: Shared dead-peer cache
The fetch layer SHALL skip peer IPs that failed to connect in any instance (≥2 failures) for ~10 minutes, using Redis when available and falling back to the in-memory cache otherwise.

#### Scenario: Fleet-wide dead IP skipped
- **WHEN** an IP failed to connect in instance A
- **THEN** instance B skips it too during the TTL

#### Scenario: Redis unavailable
- **WHEN** Redis is unreachable
- **THEN** the in-memory per-process dead-peer cache is used
