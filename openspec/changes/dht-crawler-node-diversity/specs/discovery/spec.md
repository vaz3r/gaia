## Purpose

Grow the BEP 51 node pool to thousands, sample more distinct nodes per second, and add a second infohash source (keyspace sweep + announce intake) so unique discovery rises from ~0.067% of sampled to a meaningful fraction while staying bandwidth-cheap.

## ADDED Requirements

### Requirement: Continuous node-pool growth
The crawler SHALL run a per-instance routing grower at a sub-second interval (100ms) that issues `get_peers` lookups toward random 20-byte targets so the routing table climbs toward `--max-nodes` throughout the crawl.

#### Scenario: Table climbs during steady state
- **WHEN** the crawler has been running for several minutes
- **THEN** `routing_nodes` grows from ~285 toward `--max-nodes` (4096), increasing the set of sampleable BEP 51 nodes

#### Scenario: Grower bounded by QPS budget
- **WHEN** the aggregate DHT query rate approaches `--qps`
- **THEN** growers do not exceed the budget; other query types share it fairly

### Requirement: Productivity-based node deprioritization
The sampler SHALL track per-node new-hash yield and back off nodes that return 0 new unique hashes for ~5 minutes, while keeping productive nodes on a short re-query cap.

#### Scenario: Dead BEP 51 node deprioritized
- **WHEN** a node returns 0 new unique hashes on repeated samples
- **THEN** the sampler skips it for ~5 minutes instead of re-sampling it at its advertised interval

#### Scenario: Productive node re-queried quickly
- **WHEN** a node returns new unique hashes
- **THEN** it remains sampleable at the short re-query cap (e.g. 60s max, 30s target)

### Requirement: Wide sampling spread
The sampler SHALL select targets across the full routing table via `PICK_CANDIDATES` (raised default), so queries reach many distinct BEP 51 nodes rather than a small hot set.

#### Scenario: Large table, spread queries
- **WHEN** the routing table has thousands of nodes
- **THEN** sampling queries spread across the table and surface hashes from many distinct stores

### Requirement: Keyspace get_peers node growth
The routing growers SHALL issue `get_peers` lookups to random 20-byte targets across the keyspace, feeding discovered nodes into the routing table in regions BEP 51 sampling under-weights. Uses only the stock irontide API.

#### Scenario: Sweep grows routing table
- **WHEN** the growers query random keyspace targets
- **THEN** returned nodes grow the table, expanding the BEP 51 reachable set

#### Scenario: Growth bounded by query budget
- **WHEN** the aggregate DHT query rate approaches `--qps`
- **THEN** growers do not exceed the budget; other query types share it fairly

### Requirement: Announce intake is diagnostic-only
The crawler SHALL NOT patch or vendor irontide for peer-store reading. Announced-hash volume is surfaced via the `announced_hashes` stats counter for future evaluation only.

#### Scenario: No vendored dependency
- **WHEN** the workspace builds
- **THEN** it compiles against stock crates.io irontide-dht with no `[patch.crates-io]` override

### Requirement: Cheap sampler dedup
The sampler SHALL use a ~10M-entry in-memory bloom filter to short-circuit the per-hash database `scan_blocked` check on the hot path, so dedup does not gate discovery throughput.

#### Scenario: Bloom skips DB on known hash
- **WHEN** a sampled hash is already in the bloom filter
- **THEN** the sampler skips it without a database read

#### Scenario: Bloom false positive is recoverable
- **WHEN** a rare new hash collides in the bloom filter
- **THEN** it is re-discovered in a later batch and not lost permanently

### Requirement: Batch DB triage
The crawler SHALL batch pipeline-admission database checks (~1000 hashes / 2s) instead of performing per-hash lookups, so admission keeps up with a larger unique stream.

#### Scenario: Burst of unique hashes
- **WHEN** many distinct hashes arrive at once
- **THEN** they are triaged in one batched query and admitted promptly

## MODIFIED Requirements

### Requirement: Sampler emit path
The sampler's `emit_sample` SHALL route through the bloom filter and shared seen-set before emitting, preserving the existing `min_seen` gate.

#### Scenario: Unchanged dedup semantics
- **WHEN** a hash passes min_seen and shared-dedup checks
- **THEN** it is emitted exactly as before, with bloom serving only as a fast pre-filter
