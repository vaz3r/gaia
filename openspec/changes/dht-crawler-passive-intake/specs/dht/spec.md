## Purpose

Turn the DHT node into a passive-intake citizen: absorb irontide as the owned `gaia-dht` library and expose inbound `announce_peer`/`get_peers` as a first-class event stream so the crawler consumes live hashes instead of only sampling.

## ADDED Requirements

### Requirement: Owned DHT library
The workspace SHALL contain `gaia-bencode`, `gaia-core`, `gaia-wire`, and `gaia-dht` as path-dependent workspace members (absorbed from irontide, GPL-3.0-or-later), with no `[patch.crates-io]` override.

#### Scenario: Builds against owned crates
- **WHEN** the workspace builds
- **THEN** it compiles the crawler against `vendor/gaia-*` path dependencies

### Requirement: Inbound event stream
`DhtHandle` SHALL expose `subscribe()` returning a broadcast receiver of `DhtEvent`. The actor SHALL emit `DhtEvent::Announced { info_hash, peer_addr }` for every validated inbound `announce_peer`, and `DhtEvent::LookedUp { info_hash, from_addr }` for every inbound `get_peers`.

#### Scenario: Announce surfaced to the app
- **WHEN** a remote node sends a valid `announce_peer` for hash H from peer P
- **THEN** the actor emits `Announced { info_hash: H, peer_addr: P }` on the event channel

#### Scenario: GetPeers surfaced to the app
- **WHEN** a remote node queries `get_peers` for hash H
- **THEN** the actor emits `LookedUp { info_hash: H, from_addr: <sender> }`

### Requirement: Stable node identity
Each instance SHALL persist its node ID in `node_id.json` and pass it as `DhtConfig::own_id` so the node keeps its identity across restarts and builds DHT reputation.

#### Scenario: ID stable across restarts
- **WHEN** the crawler restarts with the same state dir
- **THEN** the DHT node reuses the persisted `own_id` instead of generating a new one

### Requirement: Table growth
The DHT node SHALL support `--max-nodes` up to 8192 and `--no-restrict-ips` so the routing table grows toward thousands of nodes.

#### Scenario: Larger routing table
- **WHEN** compose passes `--max-nodes 8192 --no-restrict-ips`
- **THEN** the routing table can exceed the default one-node-per-IP cap and 512-node default
