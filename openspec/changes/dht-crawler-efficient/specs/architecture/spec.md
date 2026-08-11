## Purpose

Runs two instances (down from four) sharing a Redis-backed dedup layer, with lower DHT budgets and per-instance observability, to maximize torrents-per-byte.

## ADDED Requirements

### Requirement: Two-instance default
The PM2/Compose deployment SHALL run 2 instances by default, and the DHT/sampler query budgets SHALL default to 2000 and 400 per instance respectively.

#### Scenario: Two instances
- **WHEN** the stack starts
- **THEN** two DHT nodes run on ports 6881-6882 with per-instance budgets of 2000/400 qps

### Requirement: Optional Redis service
The stack SHALL include a `redis` service and pass its URL to the crawler; if the crawler cannot reach it, it SHALL degrade to per-instance behavior without failing.

#### Scenario: Redis present
- **WHEN** the redis service is healthy
- **THEN** the crawler uses it for the shared seen-set and dead-peer cache

#### Scenario: Redis absent or down
- **WHEN** the crawler cannot connect
- **THEN** it logs a warning and runs with in-memory dedup/cache

### Requirement: Per-instance stats
The stats output SHALL include per-instance routing-node count and sampled/unique rates so redundant instances are identifiable.

#### Scenario: Instance contribution visible
- **WHEN** the stats loop ticks
- **THEN** each instance's routing-node count and sampled/unique progress are reported
