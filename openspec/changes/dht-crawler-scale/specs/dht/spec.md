## Purpose

Grow the routing table from ~280 to thousands of nodes (K=80 + verified split policy) so distinct-node sampling can reach bitmagnet's 60/sec.

## ADDED Requirements

### Requirement: Larger routing table
The DHT SHALL keep up to `K = 80` nodes per bucket (matching bitmagnet's `nodesK`), so the table holds thousands of nodes rather than saturating at ~280.

#### Scenario: Table exceeds 280 nodes
- **WHEN** the crawler samples and runs routing growers for several minutes
- **THEN** `routing_nodes` climbs well past 280 toward thousands

#### Scenario: Lookups return more nodes
- **WHEN** a `closest(target, K)` response is produced
- **THEN** it carries up to 80 nodes, so lookups inject more nodes per response

### Requirement: Table growth not capped by split policy
If the last-bucket-only split policy would still reject legitimate inserts at K=80, full buckets SHALL split when doing so admits a closer node (mirroring bitmagnet's splittable trie).

#### Scenario: Near-bucket split admits nodes
- **WHEN** a full bucket rejects an insert but splitting would admit a closer node
- **THEN** the bucket splits and the node is inserted

## MODIFIED Requirements

### Requirement: Existing routing tests still pass
The 36 routing-table tests SHALL continue to pass with K=80; a new growth test SHALL assert the table exceeds the old ~280 ceiling.
