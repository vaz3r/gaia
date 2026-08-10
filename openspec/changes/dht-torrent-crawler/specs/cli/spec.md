## Purpose

Exposes the crawler and its index as a unix-style CLI: `run` starts the crawl daemon with configurable runtime options, and `query` searches the local database. Structured logging surfaces crawl health, throughput, and errors for 24/7 unattended operation.

## ADDED Requirements

### Requirement: Run command
The CLI SHALL provide a `run` subcommand that starts the DHT crawler, metadata fetcher, filter, and storage pipeline as a daemon until interrupted.

#### Scenario: Daemon starts and crawls
- **WHEN** the user invokes `dht-crawler run` with default options
- **THEN** the crawler bootstraps, builds its routing table, and begins emitting infohashes into the pipeline, looping until the process receives a termination signal

#### Scenario: Graceful shutdown persists state
- **WHEN** the daemon receives an interrupt/SIGTERM
- **THEN** it drains in-flight work, persists the routing table and pending database batches, and exits cleanly

### Requirement: Configurable runtime options
The CLI SHALL let the user configure the UDP bind port, the SQLite database path, the metadata-fetch concurrency, optional IPv6 support, and the DHT state directory via flags; all options SHALL have sensible defaults.

#### Scenario: Custom bind port and db path
- **WHEN** the user runs `dht-crawler run --port 6771 --db crawler.sqlite --concurrency 256`
- **THEN** the daemon binds UDP on port 6771, opens `crawler.sqlite`, and limits metadata fetches to 256 in flight, with the CLI reporting any invalid values

#### Scenario: Invalid values are rejected
- **WHEN** a flag value is invalid (e.g. a port outside 1–65535)
- **THEN** the CLI exits with a usage error and does not start the daemon

### Requirement: Query command
The CLI SHALL provide a `query` subcommand that performs a case-insensitive name search against the configured database and prints matching torrents (name, category, year, size).

#### Scenario: Results are printed
- **WHEN** the user runs `dht-crawler query "matrix 1080p"`
- **THEN** the CLI prints the matching movie/TV records with their name, category, year, and size

### Requirement: Structured logging
The CLI SHALL emit structured `tracing` logs (INFO by default, DEBUG when enabled via flag) reporting routing-table size, infohashes sampled, metadata fetched, records persisted, and errors.

#### Scenario: Throughput is logged
- **WHEN** the daemon has been running and processing infohashes
- **THEN** periodic log lines report routing-table size, sampled/unique hashes, fetch success rate, and persisted record count

#### Scenario: Errors are surfaced at ERROR level
- **WHEN** a pipeline stage encounters an unrecoverable error
- **THEN** the event is logged at ERROR level and the daemon continues running rather than exiting, unless the error is fatal