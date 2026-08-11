## Purpose

Acquires and SHA-1-verifies torrent metadata over TCP (BEP 9/10) at high concurrency: the pool is unblocked so `concurrency` truly bounds in-flight fetches, and per-hash budgets are tuned to fail fast and free slots.

## ADDED Requirements

### Requirement: Fetch concurrency matches the configured pool
The fetch layer SHALL hold a lookup permit only for the duration of starting the `get_peers` stream, then release it before dialing peers, so the number of concurrent in-flight fetches is bounded by `concurrency`, not by `lookup_concurrency`.

#### Scenario: Pool is not blocked by slow dialing
- **WHEN** many hashes have slow or unreachable peers
- **THEN** slow dialing does not consume lookup permits; the pool keeps starting new fetches up to `concurrency`

#### Scenario: Lookup count stays bounded
- **WHEN** the fetch pool is saturated
- **THEN** the number of concurrently *started* `get_peers` lookups is still bounded by `lookup_concurrency`

### Requirement: Short per-hash fetch budget
The fetch layer SHALL give each infohash a wall-clock deadline of 20 seconds and try at most 50 distinct peers, so a hash with no reachable peers frees its pool slot quickly.

#### Scenario: Dead hash frees its slot fast
- **WHEN** a hash's peers are all unreachable
- **THEN** the fetch attempt ends by the 20s deadline (or 50 peers), releasing its concurrency slot

#### Scenario: Successful fetch completes early
- **WHEN** the first dialed peer serves matching metadata
- **THEN** the fetch succeeds without waiting out the full deadline

### Requirement: Every verified torrent is kept
The fetch layer SHALL persist every torrent whose metadata passes SHA-1 verification, classifying it as `movie`, `tv`, or `other`; no verified torrent is discarded for failing classification.

#### Scenario: Unclassifiable torrent is stored
- **WHEN** verified metadata has no recognizable movie/TV pattern
- **THEN** the torrent is persisted with category `other` rather than being skipped

#### Scenario: Verified and persisted counts match
- **WHEN** the crawler reports stats
- **THEN** `records_persisted` equals `metadata_verified` (nothing verified is dropped)
