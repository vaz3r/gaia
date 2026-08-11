## Purpose

Organizes the codebase into focused modules with one-way dependencies so each concern (CLI, pipeline orchestration, discovery, fetch, classification, storage, query, purge) is separable, testable, and maintainable.

## ADDED Requirements

### Requirement: Dispatcher-style entry point
The binary's `main` SHALL only parse the CLI and dispatch to the `run`, `query`, or `purge` commands; it SHALL NOT contain pipeline wiring, storage loops, or query/purge logic.

#### Scenario: CLI dispatch only
- **WHEN** `main` runs
- **THEN** it parses arguments and calls the appropriate command handler; no crawl or storage logic lives in `main`

### Requirement: Pipeline orchestration module
A `crawler` module SHALL own the run pipeline: opening storage, starting the DHT, wiring discovery→fetch→storage channels, spawning writer/stats tasks, and coordinating graceful shutdown.

#### Scenario: Pipeline lifecycle is centralized
- **WHEN** the `run` command executes
- **THEN** all channel wiring and task lifecycle are handled by the crawler module, not by `main`

### Requirement: One-way module dependencies
Modules SHALL depend only in one direction: `crawler` depends on `discovery`, `fetch`, and `storage`; `fetch` depends on `classify`, `net`, and `storage`; no module SHALL import a module that imports it (no cycles).

#### Scenario: No circular imports
- **WHEN** the crate compiles
- **THEN** there are no module dependency cycles (enforced by structure and verified by a clean build)

### Requirement: Separation of CLI, query, and purge
CLI argument definitions (`cli`), the search command (`query`), and the data-wipe command (`purge`) SHALL be distinct modules rather than functions embedded in `main`.

#### Scenario: Query and purge are standalone
- **WHEN** `query` or `purge` is invoked
- **THEN** it is handled by its own module without touching crawler or storage-writer internals

### Requirement: Storage split
Storage SHALL be organized into at least three concerns: a facade (`mod`), schema + migrations (`schema`), and data models (`model`).

#### Scenario: Schema and models are separable
- **WHEN** the schema or a model changes
- **THEN** only the corresponding storage submodule changes; the facade API stays stable

### Requirement: Writer and stats loops live outside main
The storage writer loop and the periodic stats loop SHALL be defined in dedicated modules (e.g. within `crawler` or a `writer`/`stats` module), not in `main`.

#### Scenario: Writer and stats are not in main
- **WHEN** the source is inspected
- **THEN** `write_loop` and `stats_loop` are defined outside `main.rs`
