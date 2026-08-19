## Purpose

Describes the crawler DHT routing table's node-capacity, eviction, and growth behavior so the table can scale unbounded (to 100k+ nodes) and feed the sampler a sustained breadth of distinct nodes for infohash discovery.

## ADDED Requirements

### Requirement: Unbounded node capacity
The routing table SHALL retain every discovered node without a per-distance-region capacity ceiling, so its total node count can grow well beyond any fixed bucket-count times bucket-size product (e.g. beyond an old 160 bucket x 80 node cap of 12,800).

#### Scenario: Table grows past the old per-bucket cap
- **WHEN** nodes are discovered whose IDs map into a region that previously held only a fixed maximum of nodes
- **THEN** they are retained and the table's total node count continues to grow beyond the old ceiling

#### Scenario: No saturation in a high-density region
- **WHEN** many distinct nodes map into a single dense distance region (e.g. half the keyspace)
- **THEN** the table keeps accepting them rather than rejecting after a fixed per-region limit

### Requirement: Evict only on repeated failure
A node SHALL be removed from the routing table only when it has failed repeatedly (reached the bad-node failure threshold); it SHALL NOT be evicted merely because its distance region is full or because it is the least-recently-seen in a full bucket.

#### Scenario: A failing node is dropped
- **WHEN** a node has failed the required number of consecutive queries
- **THEN** it is evicted to make room for a new node

#### Scenario: A healthy node is not evicted for capacity
- **WHEN** new nodes are inserted but the table is under its safety ceiling
- **THEN** an existing healthy node is not dropped to make room

### Requirement: Safety ceiling on total nodes
The routing table SHALL honor a high overall node-count ceiling to bound memory, but this ceiling SHALL be large enough to admit 100k+ nodes and SHALL NOT operate as a per-region gate.

#### Scenario: Total-node ceiling bounds memory
- **WHEN** the table's total node count reaches the configured safety ceiling
- **THEN** a failing node is evicted before (or instead of) rejecting the incoming node

### Requirement: Closest-node queries stay correct
`closest(target, n)` SHALL return the `n` nodes nearest to `target` by XOR distance regardless of table size or internal structure.

#### Scenario: Nearest nodes returned at scale
- **WHEN** the table holds a very large number of nodes (100k+)
- **THEN** `closest` still returns the correct nearest `n` nodes by XOR distance

### Requirement: Full-table node access for growth and sampling
The system SHALL be able to enumerate the whole routing table (all nodes and least-recently-seen nodes) so the continuous grower and the sampler can keep cycling across all retained nodes.

#### Scenario: Grower cycles the full table
- **WHEN** the grower requests nodes for `find_node` refresh
- **THEN** it can iterate every node in an unbounded table, not just those in a few non-saturated buckets

#### Scenario: Sampler sees a wide node set
- **WHEN** the sampler selects a node to query
- **THEN** its candidate set includes nodes across the whole table, so distinct-node coverage scales with table size

### Requirement: Fastest freshness refresh supersedes bucket-target refresh
The actor's stale-region refresh SHALL not be the mechanism that keeps the table live; continuous whole-table cycling SHALL provide node freshness such that no per-bucket-target refresh loop is required for growth.

#### Scenario: Table freshness without bucket-target refresh
- **WHEN** the table is refreshed by continuous whole-table `find_node`/grower cycling
- **THEN** nodes remain live without a leading-zero-bucket-index-targeted refresh loop
