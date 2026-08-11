## Purpose

Traverses the BitTorrent DHT keyspace via BEP 51 `sample_infohashes`, fixed to stay productive: per-node re-query intervals are capped, targets spread across the routing table, and the passive-announce intake is observable so a second discovery source can be justified with data later.

## ADDED Requirements

### Requirement: Capped re-query interval
The discovery layer SHALL cap the effective per-node BEP 51 re-query interval to a configurable maximum (`--sampler-max-interval`, default 60s), regardless of the interval a node advertises, so a node advertising an hours-long interval is still re-queried regularly.

#### Scenario: Six-hour interval is capped
- **WHEN** a node responds to `sample_infohashes` with `interval = 21600` (6h)
- **THEN** the node becomes re-queryable after at most the configured cap (default 60s), not after 6h

#### Scenario: Shorter interval is honored
- **WHEN** a node responds with an interval shorter than the cap
- **THEN** the node is not re-queried until that shorter interval elapses

### Requirement: Node-selection spread
The discovery layer SHALL select a query target by picking a random ready node and using that node's own ID as the `sample_infohashes` target, shuffling the ready set before sampling so concurrent sampler loops spread across the routing table rather than converging on one node.

#### Scenario: Loops query different nodes
- **WHEN** multiple sampler loops run concurrently against a small routing table
- **THEN** the loops target distinct ready nodes, not all the same node

#### Scenario: Cooling nodes are skipped
- **WHEN** a node is inside its re-query window
- **THEN** it is not selected, and a ready node is chosen instead

#### Scenario: All nodes cooling yields no query
- **WHEN** every routing-table node is inside its re-query window
- **THEN** the sampler waits and retries rather than violating the interval

### Requirement: Productive-node bias
The discovery layer SHALL prefer ready nodes that have historically returned more samples over nodes that have failed, without allowing a few high-scoring nodes to monopolize the sampler.

#### Scenario: Productive node preferred
- **WHEN** a productive node and a failing node are both ready
- **THEN** the productive node is more likely to be selected

### Requirement: Passive announcement observability
The discovery layer SHALL surface the number of infohashes other nodes have announced to us (the actor's `peer_store_info_hashes`) in the periodic stats output, without modifying or vendoring the `irontide-dht` crate.

#### Scenario: Announced-hash count is logged
- **WHEN** the stats loop ticks
- **THEN** the log includes `announced_hashes` equal to `handle.stats().peer_store_info_hashes`

#### Scenario: No patch is required
- **WHEN** the crawler observes announcements
- **THEN** it does so through the existing `handle.stats()` API; `irontide-dht` remains a plain dependency
