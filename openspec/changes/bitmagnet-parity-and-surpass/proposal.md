## Why

While direct 1-shot KRPC peer resolution and decaying generational bloom filtering are now implemented in GAIA, our real-world indexing throughput is throttled by three operational and architectural bottlenecks:

1. **Artificial Liveness Barrier Discarding Swarms**: `docker-compose.yml` configures `--min-seen 3` and `--min-sightings 2`. In live metrics, this discards **73.5% of all sampled hashes** before fetch attempts. Bitmagnet indexes on first sighting (`min_seen = 1`), catching short-lived swarms that GAIA drops.
2. **Scale Under-provisioning**: GAIA runs with `--scale 1`, limiting fetch and discovery channel buffers, whereas Bitmagnet's proven production default is `ScalingFactor: 10`.
3. **Missing BEP 33 Scrape Filter**: Bitmagnet runs a lightweight UDP scrape stage (`requestScrape`) to verify seeders via bloom filter before dialing TCP peers, eliminating dead dials on zero-seed swarms.

## What Changes

- **First-Sighting Direct Dispatch (`first-sighting-dispatch`)**:
  - Update `docker-compose.yml` default arguments to `--min-seen 1` and `--min-sightings 1`.
  - Ensure sampled infohashes flow into triage and direct peer resolution immediately without waiting for multi-node corroboration.
- **Production Scale Tuning (`scale-tuning`)**:
  - Increase `--scale` from `1` to `10` in `docker-compose.yml`, expanding sampling loops, lookup concurrency, and pipeline channel capacities to match Bitmagnet's default concurrency.
- **Aggressive Node Re-Seeding from Peer/Sample Responses (`node-recirculation`)**:
  - Feed returned nodes from `GetPeers` and `SampleInfohashes` directly back into the sampler target pool to accelerate table growth beyond 20,000+ reachable nodes.

## Capabilities

### New Capabilities
- `first-sighting-dispatch`: Immediate queueing and fetch resolution for newly discovered hashes on first sighting.
- `scale-tuning`: High-throughput pipeline concurrency matching Bitmagnet's default scaling factor.

### Modified Capabilities
- `discovery`: Sampler emits newly observed hashes without multi-node delay gates.
- `fetch`: Concurrency and dial pools scaled to handle high-volume ingress streams.

## Impact

- **Indexing Yield**: Eliminates the 73.5% discriminator drop rate, raising verified torrent indexing rate from ~60–90/hr toward 1,000–3,000+/hr.
- **Routing Network**: Accelerates routing table expansion past 20,000 nodes via continuous response recirculation.
