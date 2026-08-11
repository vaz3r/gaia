## Purpose

Maximize unique infohash discovery per unit of bandwidth: share the seen-set across instances, spread sampling across the full routing table, and let get_peers lookups double as node discovery.

## ADDED Requirements

### Requirement: Shared seen-set
The discovery layer SHALL consult a shared Redis `SEEN` set when emitting an infohash: a hash already emitted by any instance SHALL be skipped locally. If Redis is unreachable, the crawler SHALL fall back to per-instance in-memory dedup.

#### Scenario: Fleet-wide dedup
- **WHEN** instance A emits hash X and instance B later samples X
- **THEN** B skips X rather than emitting it again

#### Scenario: Redis unavailable
- **WHEN** the Redis URL is unreachable
- **THEN** the crawler uses per-instance `SeenCounts` and continues normally

### Requirement: Full-table sampling spread
The sampler SHALL pick targets across the full routing table rather than repeatedly selecting a small set of recently-used nodes, so more distinct BEP 51 nodes are reached.

#### Scenario: Distinct nodes queried
- **WHEN** the routing table has many nodes
- **THEN** sampling queries spread across the table, reaching distinct BEP 51 nodes and surfacing new hashes

### Requirement: get_peers doubles as discovery
The fetch layer SHALL rely on get_peers DhtLookups feeding discovered nodes into the routing table, so metadata lookups also grow the node pool at no extra cost.

#### Scenario: Lookups grow routing
- **WHEN** a metadata get_peers lookup runs
- **THEN** discovered nodes are added to the routing table, expanding the BEP 51 reachable set
