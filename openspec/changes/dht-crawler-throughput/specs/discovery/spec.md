## Purpose

Widen discovery breadth by running several independent DHT nodes/samplers against one database, and grow the routing table faster at startup with targeted lookups.

## ADDED Requirements

### Requirement: Multi-instance crawling
The crawler SHALL accept `--instances N` (default 1). Each instance SHALL bind UDP on `port + i`, use state directory `state-dir/instance-i/`, and run its own sampler; all instances SHALL share one storage handle and one fetch pool.

#### Scenario: Two instances double discovery
- **WHEN** `--instances 2` is given
- **THEN** two DHT nodes run on distinct ports and state dirs, each sampling independently, writing to the same database

#### Scenario: Single instance is unchanged
- **WHEN** `--instances 1` (default) is given
- **THEN** behavior matches a single-node crawl on `port` and `state-dir`

### Requirement: Routing table warmup
The discovery layer SHALL issue ~16 throttled `get_peers` queries on random targets at startup to populate the routing table faster, before handing off to the normal sampler.

#### Scenario: Cold start populates quickly
- **WHEN** the crawler starts with an empty routing table
- **THEN** the warmup phase issues targeted `get_peers` lookups to grow the table beyond the bootstrap baseline

#### Scenario: Warmup respects the query budget
- **WHEN** the warmup phase runs
- **THEN** its queries count against the configured per-second budget
