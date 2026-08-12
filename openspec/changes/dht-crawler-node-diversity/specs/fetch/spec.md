## Purpose

Tune the fetch pipeline so it churns dead peers faster and keeps up with a larger stream of distinct infohashes produced by node-pool growth and the second hash source, without letting bandwidth balloon.

## ADDED Requirements

### Requirement: Faster dead-peer timeout
The fetch layer SHALL reduce `FETCH_TIMEOUT` to 5s so failed/quiet peers free pool slots quickly, while retaining the `empty_peers` fast-path and the shared dead-peer cache.

#### Scenario: Dead peer frees slot quickly
- **WHEN** a dialed peer does not complete within 5s
- **THEN** the dial fails, is counted as a timeout, and the pool slot is reused

#### Scenario: Known-dead IP skipped
- **WHEN** an IP is flagged dead fleet-wide
- **THEN** the fetch skips it without dialing, preserving bandwidth

### Requirement: Unique-hash rate stats
The stats line SHALL report the unique-hash discovery **rate** (unique/hr) alongside the existing sampled total, so the discovery levers (Phase A/B) are observable independently of fetch success.

#### Scenario: Discovery lever visibility
- **WHEN** the node pool grows or a hash source is added
- **THEN** the unique/hr stat changes, making the effect measurable

### Requirement: Per-source counters
The stats line SHALL track how many hashes were discovered by BEP 51 sampling vs keyspace sweep vs announce intake.

#### Scenario: Source attribution
- **WHEN** the crawler discovers hashes from multiple sources
- **THEN** counters attribute each hash to its source so ineffective sources can be disabled

## MODIFIED Requirements

### Requirement: Fetch pool saturation
The fetch layer SHALL keep `concurrency` (512) and `lookup_concurrency` (256) effective as the candidate stream grows; if the queue drains, the pool SHALL scale to the larger stream without per-hash wall-clock stalls.

#### Scenario: Larger stream keeps pool busy
- **WHEN** unique discovery rises 3-10x
- **THEN** the pool stays saturated and records_persisted/hr rises proportionally
