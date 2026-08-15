## Purpose

Periodically persist the full crawler and system resource snapshot so crawler behavior and host health are queryable over time by the admin API and dashboard.

## ADDED Requirements

### Requirement: Periodic full crawl snapshot
The crawler SHALL write one row to `crawl_stats_history` every 30 seconds containing the complete set of crawl counters surfaced by the stats loop: sampled/unique/announced hashes, liveness-gate and shadow counters, fetch attempts/failures, verified totals and per-source splits, terminal-dead count, pipeline depth snapshots, per-peer failure taxonomy, and DHT actor diagnostics.

#### Scenario: Snapshot row appears on tick
- **WHEN** 30 seconds elapse while the crawler runs
- **THEN** a `crawl_stats_history` row is written containing all crawl counter fields

#### Scenario: Counters are cumulative
- **WHEN** consecutive snapshots are compared
- **THEN** cumulative counters never decrease between rows

### Requirement: System resource metrics persisted
Each snapshot SHALL include system metrics: tunnel (tun0) network bytes received/sent with derived rates (bytes/sec), host and container memory usage, CPU utilization percent, disk usage, and load average.

#### Scenario: Bandwidth tracks tunnel traffic
- **WHEN** the crawler is actively sampling and fetching
- **THEN** the snapshot's network byte counters increase and derived rates are positive

#### Scenario: Resource metrics populated each tick
- **WHEN** any snapshot is inspected
- **THEN** CPU percent, memory, disk, and load-average fields are present and sane (non-negative, bounded)

### Requirement: History queryable by admin API
The admin API SHALL be able to read snapshot history over arbitrary time ranges and derive windowed rates (e.g. verified/hr, unique/hr, bandwidth) from the persisted rows.

#### Scenario: Range query returns ordered rows
- **WHEN** the admin API requests history for a time range
- **THEN** it returns the snapshots in time order, bounded by the requested range and a limit

#### Scenario: Rates derived in SQL
- **WHEN** a rate metric (e.g. verified/hr) is requested
- **THEN** it is computed from consecutive snapshot deltas (window functions), not from the crawler

### Requirement: Monitoring without crawler writes
A read-only failure of the stats persistence SHALL NOT break the crawl: if a snapshot write fails, the crawler SHALL continue crawling and logging.

#### Scenario: Postgres hiccup during stats write
- **WHEN** a stats snapshot insert fails transiently
- **THEN** the crawler logs the error and continues the crawl loop
