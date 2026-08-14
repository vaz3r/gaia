## Purpose

Grows each DHT instance's routing table continuously so more BEP 51-capable nodes are discovered, raising the unique infohash discovery rate and thus torrent throughput.

## ADDED Requirements

### Requirement: Continuous routing growth
Each DHT instance SHALL run a background grower task that continuously issues `get_peers` on random 20-byte targets at a throttled interval, so newly-discovered nodes are injected into that instance's routing table throughout the crawl.

#### Scenario: Routing table keeps growing
- **WHEN** the crawler runs for an extended period
- **THEN** the routing table size climbs toward the configured `--max-nodes` cap rather than stalling after startup

#### Scenario: Grower respects the query budget
- **WHEN** the grower is active
- **THEN** its `get_peers` queries are throttled and count against the shared DHT per-second budget

#### Scenario: Grower stops on shutdown
- **WHEN** the crawler receives SIGTERM/SIGINT
- **THEN** grower tasks stop alongside the samplers and do not block drain

### Requirement: Configurable routing table cap
The crawler SHALL expose the routing table cap via `--max-nodes` with a default of 4096 (8192 under `--aggressive`).

#### Scenario: Cap has headroom
- **WHEN** a growing routing table approaches the previous default of 2048
- **THEN** the new default of 4096 permits continued growth without hitting a hard stop

### Requirement: Optional IP restriction lift
The crawler SHALL expose `--no-restrict-ips` which disables irontide's one-node-per-IP routing restriction, defaulting to the restricted behavior.

#### Scenario: Restricted by default
- **WHEN** `--no-restrict-ips` is not given
- **THEN** the routing table keeps at most one node per IP

#### Scenario: Restriction lifted
- **WHEN** `--no-restrict-ips` is given
- **THEN** multiple nodes sharing an IP may both be admitted to the routing table
