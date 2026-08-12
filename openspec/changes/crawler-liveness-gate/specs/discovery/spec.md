## Purpose

Gate sampled-hash fetches on N distinct DHT nodes reporting the hash within a rolling window, with correct cross-loop semantics and validation-first shadow mode.

## ADDED Requirements

### Requirement: Shared distinct-source counter
The discovery layer SHALL maintain one per-process counter shared across all sampler loops, keyed by hash with a per-hash list of `(source_node_id, report_time)` reports, upserted by source node ID (a repeat from the same node updates its timestamp in place, never a new slot).

#### Scenario: Cross-loop distinctness
- **WHEN** two different sampler loops each report the same hash from different DHT nodes
- **THEN** the counter records two distinct sources

#### Scenario: Same-source repeat
- **WHEN** the same DHT node reports the same hash again within the window
- **THEN** only its timestamp updates; the distinct-source count is unchanged

### Requirement: Liveness emission threshold
A sampled hash SHALL be emitted to the fetcher only when the count of distinct sources with reports within the window reaches `--min-seen`.

#### Scenario: Below threshold
- **WHEN** a hash has fewer than `--min-seen` distinct sources within the window
- **THEN** it is not fetched

#### Scenario: Threshold reached
- **WHEN** a hash reaches `--min-seen` distinct sources within the window
- **THEN** it is emitted (subject to the bloom/DB/Redis gates)

### Requirement: Hinted hashes exempt
Announce-derived hashes carrying a live peer hint SHALL bypass the liveness gate.

#### Scenario: Announced hash fetched immediately
- **WHEN** a hash arrives with a live announce peer hint
- **THEN** it is fetched regardless of sighting count

## MODIFIED Requirements

### Requirement: Rolling window expiry
A report older than `--liveness-window` SHALL expire on encounter, and a hash whose reports all expire SHALL be evicted.

#### Scenario: Expired report ignored
- **WHEN** a report's timestamp is older than the window
- **THEN** it no longer counts toward the distinct-source total

#### Scenario: All reports expired
- **WHEN** every report for a hash has expired
- **THEN** the hash entry is removed

### Requirement: Global backstop
The counter SHALL enforce `--liveness-max-entries` (default 100k, oldest-first) via a periodic sweep so one-hit-wonder hashes that are never re-read cannot accumulate unboundedly.

#### Scenario: Backstop eviction
- **WHEN** the counter exceeds `--liveness-max-entries`
- **THEN** the oldest entries are evicted
