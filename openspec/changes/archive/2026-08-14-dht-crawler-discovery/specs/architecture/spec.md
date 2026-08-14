## Purpose

Runs multiple independent DHT instances (default 4 under PM2) so discovery breadth multiplies while the fetch pool and database remain shared.

## ADDED Requirements

### Requirement: PM2 runs multiple instances
The PM2 ecosystem config SHALL run the crawler with `--instances 4`, binding UDP ports `6881`–`6884` with independent routing tables and samplers feeding one database.

#### Scenario: Four instances start
- **WHEN** `pm2 start ecosystem.config.cjs` runs
- **THEN** four DHT nodes start on ports 6881–6884, each with its own `state-dir/instance-N/` routing state, all sharing `crawler.sqlite`

#### Scenario: Single instance is still supported
- **WHEN** an operator passes `--instances 1`
- **THEN** behavior matches a single-node crawl on `--port` and `--state-dir`

### Requirement: Shared fetch pool and database
All instances SHALL emit discovered hashes into one shared fetch pool and persist through one shared storage writer; the writer SHALL remain single so SQLite WAL concurrency stays safe.

#### Scenario: Instances feed one pipeline
- **WHEN** several instances are running
- **THEN** their samplers all emit into the same hash channel, and records are written by one storage writer
