## Purpose

Bootstraps and maintains a persistent Kademlia routing table and traverses the BitTorrent DHT keyspace using BEP 51 `sample_infohashes`, emitting unique infohashes into the crawl pipeline. This capability is what makes a lightweight indexer efficient: a single UDP node can survey the whole DHT within hours, in compliance with the DHT protocols.

## ADDED Requirements

### Requirement: Node bootstrap
The crawler SHALL bootstrap its Kademlia routing table by querying a configured set of well-known bootstrap nodes on startup (e.g. `router.bittorrent.com:6881`, `dht.transmissionbt.com:6881`, `dht.libtorrent.org:25401`) and progressively discover neighbouring nodes via iterative `find_node`.

#### Scenario: Cold start discovers neighbours
- **WHEN** the crawler starts with an empty routing table
- **THEN** it queries the configured bootstrap nodes and iteratively populates its routing table until bootstrap completes or the bootstrap timeout elapses

#### Scenario: Bootstrap failure is reported
- **WHEN** no bootstrap node responds within the configured timeout
- **THEN** the crawler logs a warning and continues attempting periodic bootstrap, never panicking

### Requirement: Persistent routing table
The crawler SHALL persist its routing table to disk and reload it on startup, so that restarts resume crawling from a warm table instead of starting from zero.

#### Scenario: Routing table survives restart
- **WHEN** the crawler shuts down cleanly after building a routing table
- **THEN** the table is saved to the configured state directory and reloaded on the next start

#### Scenario: Corrupt or absent state does not crash
- **WHEN** the persisted routing table file is missing or unparseable
- **THEN** the crawler starts with an empty table and bootstraps from scratch without erroring out

### Requirement: Keyspace traversal via BEP 51
The crawler SHALL issue BEP 51 `sample_infohashes` queries across random 20-byte key-space targets, using the `nodes` entries returned in each response to keep discovering BEP 51-capable nodes to query.

#### Scenario: Sample across the keyspace
- **WHEN** the crawler has a populated routing table
- **THEN** it issues periodic `sample_infohashes` queries against rotating random targets and feeds returned nodes back into the routing table

#### Scenario: Response interval is honored
- **WHEN** a node returns a `sample_infohashes` response with an `interval` field
- **THEN** the crawler does not re-query that node until the interval has elapsed

### Requirement: Unique infohash emission
The crawler SHALL emit each sampled infohash into the pipeline exactly once, deduplicating against hashes already emitted or already persisted.

#### Scenario: Duplicate samples are suppressed
- **WHEN** the same 20-byte infohash is sampled from multiple nodes
- **THEN** the pipeline receives only the first occurrence, and later duplicates are dropped

### Requirement: Rate-limited queries
The crawler SHALL bound its aggregate query rate so it does not flood nodes, using per-node interval backoff and a configurable global query concurrency limit.

#### Scenario: Global limit constrains concurrency
- **WHEN** the configured per-second query budget is reached
- **THEN** additional queries are queued or deferred, never sent beyond the budget

### Requirement: Crawler node is UDP-light
The DHT crawler layer SHALL communicate exclusively over UDP (BEP 5 KRPC), and SHALL NOT open TCP connections to DHT nodes; TCP is reserved for the separate metadata-enrichment stage.

#### Scenario: No TCP from crawler layer
- **WHEN** the crawler performs DHT sampling and routing operations
- **THEN** it uses UDP datagrams only, and establishing a TCP socket is not part of this capability