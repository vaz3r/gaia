# Capability: First Sighting Dispatch

## Requirements

### Requirement: Zero-Delay Swarm Dispatch
The crawler sampler SHALL emit newly sampled infohashes immediately to the fetcher pipeline on their first sighting (`min_seen = 1`, `min_sightings = 1`) without waiting for multi-node corroboration or repeat sightings.

#### Scenario: Hash dispatched on first discovery
- **Given** an infohash `H` reported for the first time by a DHT node
- **When** `H` passes the in-memory generational bloom filter
- **Then** `H` is immediately forwarded to the fetch queue with its reporting node address attached
- **And** no discriminator drop or liveness counter delay is applied.

### Requirement: Scaled Concurrent Pipeline
The crawler SHALL scale its channel buffers and concurrent worker loops proportionally with `--scale` to prevent pipeline stalls during high-volume ingress.

#### Scenario: Scaled channel capacity under burst sampling
- **Given** `--scale 10` is configured
- **When** high-volume sample responses arrive
- **Then** the sampler channel buffer holds up to 81,920 items without dropping or blocking.
