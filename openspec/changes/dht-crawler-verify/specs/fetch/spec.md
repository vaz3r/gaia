## Purpose

Raises the fraction of fetched hashes that verify by trying more peers per infohash and giving slow-but-live peers more time, exploiting the fetch pool's idle capacity.

## ADDED Requirements

### Requirement: Higher dial concurrency per hash
The fetch layer SHALL dial at least 32 peers concurrently per infohash batch.

#### Scenario: More peers dialed in parallel
- **WHEN** a hash has many peers
- **THEN** up to 32 are dialed at once (previously 16), raising the chance one answers within the deadline

### Requirement: Higher per-hash peer budget
The fetch layer SHALL try up to 100 distinct peers per infohash before giving up.

#### Scenario: More peers tried per hash
- **WHEN** a hash's early peers are unreachable
- **THEN** the fetch continues through more peers (up to 100) before failing

### Requirement: Longer per-hash and per-peer budgets
The fetch layer SHALL allow up to 20s per infohash, 10s per peer, and 5s per TCP connect.

#### Scenario: Slow-but-live peer succeeds
- **WHEN** a peer answers slowly (e.g. 6–10s)
- **THEN** the fetch accepts it instead of timing out at the previous 7s limit

#### Scenario: Deadline still bounds the fetch
- **WHEN** a hash has not succeeded after 20s
- **THEN** the fetch ends and its slot is freed

### Requirement: Early abort stays proportionate
The fetch layer SHALL abort a hash early after 64 consecutive failed dials with no successful handshake.

#### Scenario: Dead hash still frees its slot quickly
- **WHEN** a hash's peers are all unreachable
- **THEN** the fetch aborts after the early-abort threshold rather than waiting the full 20s
