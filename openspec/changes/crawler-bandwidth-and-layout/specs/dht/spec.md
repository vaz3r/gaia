## Purpose

Remove the 10x UDP response inflation introduced by K=80 without reducing routing-table capacity or discovery.

## ADDED Requirements

### Requirement: Response payload size decoupled from table capacity
The DHT SHALL return at most `RESPONSE_K` (16) nodes in inbound-query responses, while the routing table SHALL keep `K=80` for capacity.

#### Scenario: FindNode response bounded
- **WHEN** a remote node sends `find_node`
- **THEN** the response carries at most 16 closest nodes, not 80

#### Scenario: GetPeers no-peers branch bounded
- **WHEN** a `get_peers` query finds no stored peers
- **THEN** the response carries at most 16 closer nodes

#### Scenario: BEP44 GetItem responses bounded
- **WHEN** a `get` query is answered with closer nodes
- **THEN** the response carries at most 16 nodes

#### Scenario: SampleInfohashes response bounded
- **WHEN** a remote node sends `sample_infohashes`
- **THEN** the response carries at most 16 closer nodes

#### Scenario: Table capacity unchanged
- **WHEN** the routing table ingests discovered nodes
- **THEN** it still holds up to K=80 nodes per distance bucket (thousands total)

## MODIFIED Requirements

### Requirement: Routing-table tests unchanged
The gaia-dht suite SHALL still pass with table K=80 and RESPONSE_K=16.
