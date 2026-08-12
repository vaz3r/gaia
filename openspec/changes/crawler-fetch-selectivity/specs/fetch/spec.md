## Purpose

Halve fetch volume and cap per-fetch download so bandwidth tracks verification, not dead-hash churn.

## ADDED Requirements

### Requirement: Two-sighting fetch gate
The fetch layer SHALL only fetch a sampled infohash after 2+ distinct BEP 51 responses reported it (`--min-seen` default 2). Announce-derived hashes carrying a live peer hint SHALL remain exempt.

#### Scenario: Single-sighting hash deferred
- **WHEN** a sampled hash has been reported by exactly one node
- **THEN** it is not fetched until a second distinct node reports it

#### Scenario: Hinted hash fetched immediately
- **WHEN** a hash arrives with a live announce peer hint
- **THEN** it is fetched immediately regardless of sighting count

### Requirement: Incremental metadata download
The fetch layer SHALL request metadata pieces one at a time (piece N only after piece N-1's data arrives) rather than requesting all pieces upfront, so a stalled peer costs at most a small partial download.

#### Scenario: Stalled peer costs little
- **WHEN** a peer advertises ut_metadata but stops sending data
- **THEN** the fetch aborts after ~1-2 pieces instead of downloading the whole metadata

#### Scenario: Live peer still completes
- **WHEN** a peer serves metadata pieces promptly
- **THEN** the incremental requests assemble the full metadata and SHA-1 verification proceeds unchanged
