## Purpose

Describe the structural changes to the process/CLI layout that support node diversity. The irontide stack stays stock (no vendoring): node growth and dedup use only the existing public API.

## ADDED Requirements

### Requirement: Stock irontide dependency
The workspace SHALL depend on crates.io `irontide-dht` without a `[patch.crates-io]` override or vendored copy, so upstream upgrades are drop-in.

#### Scenario: Build uses registry irontide
- **WHEN** the workspace builds
- **THEN** it compiles against the registry irontide-dht with no local override

### Requirement: In-memory bloom filter
The crawler SHALL keep a ~10M-entry in-memory bloom filter shared across sampler loops to short-circuit database reads for known-blocked hashes.

#### Scenario: Bloom caches terminal skip verdicts
- **WHEN** a hash is confirmed accepted/filtered by the database
- **THEN** later re-sightings skip the database read entirely

### Requirement: Background routing growth
The crawler SHALL spawn one routing grower per instance at a 100ms interval issuing random-target `get_peers` lookups, growing each table toward `--max-nodes` throughout the crawl.

#### Scenario: Growers run alongside sampling
- **WHEN** the sampler and fetch pool are active
- **THEN** the growers contribute node growth without blocking either

#### Scenario: Growers shut down cleanly
- **WHEN** the crawler receives SIGTERM/SIGINT
- **THEN** growers stop with the other tasks and routing state persists as usual
