# Design: Bitmagnet Parity & Surpassing Strategy

## 1. First-Sighting Dispatch vs Artificial Liveness Filtering

### Context
In earlier iterations of GAIA, a discriminator was added to reduce DB load by requiring $\ge 2$ or $\ge 3$ sightings before fetching. However, Bitmagnet avoids DB load using its in-memory Bloom filter (`ignoreHashes`), allowing it to fetch on **first sighting ($min\_seen = 1$)**. 

Because Bitmagnet fetches immediately, it catches active swarm peers before they disconnect. GAIA's delay meant that by the time 3 nodes reported a hash, the swarm was often cold or peers were dead.

### Architecture
1. In `docker-compose.yml`, change `--min-seen 3` → `--min-seen 1` and `--min-sightings 2` → `--min-sightings 1`.
2. The generational bloom filter (`GenerationalBloom`) and Redis dedup layer (`seen_contains`) handle deduplication efficiently without DB overload.

---

## 2. Scale Factor & Channel Sizing

### Bitmagnet Parity
Bitmagnet multiplies channel capacities and worker goroutines by `ScalingFactor = 10`.
GAIA's `RunArgs` supports `--scale`. Raising `--scale` to `10` in compose increases:
- Sampler loops: $32 \times 10 = 320$ loops across 8 instances
- Sampler QPS: $400 \times 10 = 4,000$ QPS aggregate
- Fetch Concurrency: $512 \times 10 = 5,120$ max concurrent in-flight fetches
- Channel Buffers: $8,192 \times 10 = 81,920$ items

---

## 3. Continuous Routing Table Node Recirculation

### Bitmagnet Logic
Every `get_peers` and `sample_infohashes` response contains `nodes` / `nodes6`. Bitmagnet asynchronously feeds these nodes to `discoveredNodes` with 1-second timeout non-blocking pushes.

### GAIA Integration
Ensure `checked_insert` in `gaia-dht` immediately updates bucket liveness and feeds the crawler's shared Redis node pool so all 8 instances cross-pollinate new nodes continuously.
