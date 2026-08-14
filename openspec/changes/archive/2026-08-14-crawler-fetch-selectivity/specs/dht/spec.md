## Purpose

Describe the CLI change that gates fetch volume on hash corroboration.

## ADDED Requirements

### Requirement: min_seen default 2
The CLI SHALL default `--min-seen` to 2 so sampled hashes require two distinct BEP 51 sightings before being fetched.

#### Scenario: Default value
- **WHEN** `--min-seen` is not provided
- **THEN** the sampler emits a sampled hash only after 2 sightings

### Requirement: Hinted intake exempt
The passive-announce intake SHALL emit hinted requests with a single occurrence and the fetcher SHALL accept them regardless of `--min-seen`.

#### Scenario: Announced hash bypasses min_seen
- **WHEN** the intake forwards an announced hash with a peer hint
- **THEN** it is fetched even though its occurrence count is 1
