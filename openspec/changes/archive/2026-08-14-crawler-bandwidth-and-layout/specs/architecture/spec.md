## Purpose

Restructure the workspace to `crawler/` with internal `crawler/crates/gaia-*` members, keeping the durable data volume stable across the rename.

## ADDED Requirements

### Requirement: App directory renamed
The crawler app SHALL live in `crawler/` (renamed from `dht-crawler/`), and its owned library crates SHALL live in `crawler/crates/gaia-*` (moved from root `vendor/`).

#### Scenario: Workspace builds after rename
- **WHEN** `cargo build --release -p crawler` runs
- **THEN** it compiles the crawler and its `crawler/crates/gaia-*` path dependencies

### Requirement: Data volume stable
The compose volume holding the crawl database and state SHALL keep its existing name (dht-crawler-data) so data and node identity persist across the rename.

#### Scenario: Data persists across rename
- **WHEN** the renamed stack deploys
- **THEN** it reuses the existing volume and the indexed torrents + node IDs remain

### Requirement: References updated
All references to the old `dht-crawler` folder/binary/container SHALL be updated (Dockerfile COPY paths, compose service/container names, run.sh, ecosystem.config.cjs, .gitignore, .env.example, cli binary name + RUST_LOG filter, wire client-id string, benchmark scripts, openspec config).

#### Scenario: No stale references
- **WHEN** the repo is grepped for the old layout paths
- **THEN** only historical docs/openspec archives reference them
