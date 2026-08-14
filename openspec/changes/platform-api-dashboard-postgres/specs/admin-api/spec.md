## Purpose

A read/write admin API exposing crawler monitoring, configuration, and service health for operators and the dashboard.

## ADDED Requirements

### Requirement: Health endpoints
The admin API SHALL expose health status for each service in the platform (postgres, redis, crawler, api) reporting reachability and, where available, liveness.

#### Scenario: All services healthy
- **WHEN** all platform services are up and reachable
- **THEN** the health endpoint reports each service as healthy

#### Scenario: A service is down
- **WHEN** one service (e.g. redis) is unreachable
- **THEN** the health endpoint reports that service as unhealthy while others remain reported accurately

### Requirement: Monitoring read endpoints
The admin API SHALL expose: latest crawl snapshot, history over a range with selectable metric(s), failure breakdown by reason over a range, and system resource history (network, cpu, memory, disk).

#### Scenario: Latest snapshot
- **WHEN** the latest snapshot is requested
- **THEN** it returns the most recent `crawl_stats_history` row with all fields

#### Scenario: History with range and metric
- **WHEN** history is requested for a range and a specific metric
- **THEN** it returns that metric's time series in order with derived rates where applicable

#### Scenario: Failure breakdown
- **WHEN** failures are requested over a range
- **THEN** it returns counts grouped by failure reason, sorted descending

#### Scenario: System resource history
- **WHEN** system history is requested over a range
- **THEN** it returns network, cpu, memory, and disk time series for that range

### Requirement: Configuration management
The admin API SHALL support reading and updating arbitrary key/value configuration stored in the `app_config` table (JSONB values), with per-key upsert and list/read operations.

#### Scenario: Read a config key
- **WHEN** an existing config key is requested
- **THEN** the API returns its JSONB value

#### Scenario: Upsert a config key
- **WHEN** a config key is written
- **THEN** the key is created or its value replaced, and a subsequent read returns the new value

### Requirement: Validation and error semantics
Admin API responses SHALL use consistent JSON error shapes (error message + optional details) and SHALL reject invalid input with 400.

#### Scenario: Invalid range parameter
- **WHEN** a request specifies an unsupported time range
- **THEN** the API responds 400 with a clear error message
