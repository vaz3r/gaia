# Capability: Direct Peer Resolution

## Requirements

### Requirement: Direct 1-Shot get_peers RPC
The DHT subsystem SHALL provide an API to query a specific remote DHT node for peers of an infohash in a single direct KRPC request without spawning an iterative graph walk.

#### Scenario: Direct peer query returns compact peers
- **Given** a known responsive node `N` and infohash `H`
- **When** `direct_get_peers(N, H)` is invoked
- **Then** a single `get_peers` KRPC query is sent to `N`
- **And** any returned compact peer addresses are parsed and returned to the caller within a single RTT.

### Requirement: Fetch Pipeline Prioritizes Direct Reporting Node
When a sampled infohash includes the address of the node that reported it, the fetch pipeline SHALL query that reporting node directly before initiating any multi-hop DHT lookup.

#### Scenario: Direct resolution eliminates empty_peers failure
- **Given** an infohash `H` sampled from node `N`
- **When** the fetch worker starts processing `H`
- **Then** it queries `N` directly for peers
- **And** if `N` returns peers, those peers are immediately queued for metadata wire fetching.
