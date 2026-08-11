## Purpose

A metadata fetch pool that runs at full concurrency, fails dead hashes fast, and avoids re-dialing unreachable peers, so the pool spends its slots on hashes that can actually verify.

## ADDED Requirements

### Requirement: Unblocked lookup starts
The fetch layer SHALL bound concurrent `get_peers` lookups by `lookup_concurrency` with a default of 256 (512 under `--aggressive`) and a DHT query budget of 8000 (12000 under `--aggressive`), and SHALL log `fetch_in_flight` and `queue_depth` so the pool's utilization is observable.

#### Scenario: Pool runs near saturation
- **WHEN** the crawler has a large hash backlog
- **THEN** the number of in-flight fetches approaches `concurrency`, and the stats log reports a rising `fetch_in_flight` and a draining `queue_depth`

#### Scenario: Lookup count stays bounded
- **WHEN** the pool is saturated
- **THEN** concurrently *started* `get_peers` lookups never exceed `lookup_concurrency`

### Requirement: Early dead-hash abort
The fetch layer SHALL abort a per-hash fetch once the first ~24 dials have all failed to connect or handshake (no successful handshake among them), instead of waiting out the full deadline.

#### Scenario: Dead hash frees its slot quickly
- **WHEN** a hash's first peers are all unreachable
- **THEN** the fetch ends after the early-abort condition rather than the full deadline

#### Scenario: Live hash is not prematurely aborted
- **WHEN** at least one early dial completes a handshake
- **THEN** the fetch continues trying peers until success or the deadline

### Requirement: Short per-hash deadline
The fetch layer SHALL give each infohash a wall-clock budget of 12 seconds.

#### Scenario: Deadline caps fetch time
- **WHEN** a hash has not succeeded after 12s
- **THEN** the fetch ends and its slot is freed

### Requirement: In-run dead-peer cache
The fetch layer SHALL remember IPs that failed to connect (≥2 failures), skip them when dialing other hashes for up to ~10 minutes, and allow them again after the TTL.

#### Scenario: Dead IP is not re-dialed
- **WHEN** an IP failed to connect for a previous hash within the TTL window
- **THEN** it is skipped for subsequent hashes

#### Scenario: Recovered IP is retried after TTL
- **WHEN** the TTL for a skipped IP has elapsed
- **THEN** the IP becomes a dial candidate again
