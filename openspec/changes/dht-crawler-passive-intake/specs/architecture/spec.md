## Purpose

Describe the structural/process changes: four owned library crates in the workspace, per-instance passive-intake subscribers, and stable node identity.

## ADDED Requirements

### Requirement: Workspace owns the DHT library
The workspace SHALL list `vendor/gaia-bencode`, `vendor/gaia-core`, `vendor/gaia-wire`, and `vendor/gaia-dht` as members, with the crawler depending on them by path.

#### Scenario: Single `cargo build` compiles everything
- **WHEN** `cargo build --release -p dht-crawler` runs
- **THEN** it compiles the four `gaia-*` crates and the crawler in one workspace

### Requirement: Per-instance passive-intake subscriber
The crawler SHALL spawn one `run_passive_intake` task per DHT instance, each subscribing to that instance's `DhtEvent` stream and sharing the Redis seen-set for dedup.

#### Scenario: Intake runs per instance
- **WHEN** the crawler starts with N instances
- **THEN** N intake tasks subscribe and forward announces to the shared pipeline

#### Scenario: Intake shuts down cleanly
- **WHEN** the crawler receives SIGTERM/SIGINT
- **THEN** intake tasks stop with the samplers and growers, and routing state persists

### Requirement: Stable identity in state dir
Each instance SHALL load-or-create `node_id.json` in its state dir and pass the ID to `DhtConfig::own_id`.

#### Scenario: First run creates the ID
- **WHEN** an instance starts with no `node_id.json`
- **THEN** it generates, persists, and uses a new node ID
