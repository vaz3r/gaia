## Why

GAIA currently indexes ~24 to 62 torrents/hour (averaging ~45/hr) with an overall failure rate of 99.89% (6.34M out of 9.15M failed fetches are `empty_peers`). An in-depth review of Bitmagnet (`internal/dhtcrawler`) reveals that Bitmagnet achieves thousands of torrents/hr by using:
1. **Direct Peer Resolution**: Preserving the `(hash, reporting_node)` pair from `sample_infohashes` and querying that node directly with a single-shot `get_peers` RPC instead of launching a 64-node iterative Kademlia walk.
2. **Stable/Decaying Bloom Filtering**: Using an aging bloom filter that evicts older entries instead of permanently blacklisting dead hashes.
3. **Opportunistic Inbound Node Harvesting**: Feeding all inbound KRPC message headers directly into routing tables to sustain 50k+ active nodes.
4. **Decoupled Concurrency Pipeline**: Isolating DHT sampling, DB triage, KRPC peer resolution, TCP wire fetching, and DB writes into dedicated bounded channels.

## What Changes

- **Direct 1-shot Peer Resolution (`direct-peer-resolution`)**:
  - `gaia-dht` adds `direct_get_peers(target_addr, info_hash)` executing a single-packet KRPC `get_peers` query directly to `target_addr`.
  - `discovery::sampler` pairs every sampled infohash with its responding node address (`node_addr`).
  - `fetch` attempts direct peer resolution against `reporting_node` first; if peers are returned, it immediately dials them without running an iterative tree walk.
- **Aging / Decaying Bloom Filter (`decaying-bloom-filter`)**:
  - Replace static `seen_bloom` with an aging generational/decaying filter in `crawler/src/bloom.rs`.
  - Stop permanently marking `terminal_dead` hashes in the bloom filter.
- **Opportunistic Node Harvesting (`inbound-node-harvesting`)**:
  - In `gaia-dht/src/actor.rs`, harvest every inbound sender ID & IP address from valid KRPC messages into a discovered nodes channel to continually refresh routing tables.
- **Decoupled Multi-Stage Channels (`channel-decoupled-pipeline`)**:
  - Isolate sampling, deduplication triage, peer resolution, metadata wire fetching, and storage persistence into independent buffered channels.

## Capabilities

### New Capabilities
- `direct-peer-resolution`: Direct single-RPC `get_peers` to the reporting node from BEP 51 samples.
- `decaying-bloom-filter`: Time-decaying / generational bloom filter that allows resurfaced torrents to be retried after an eviction window.
- `inbound-node-harvesting`: Continuous ingestion of active DHT nodes from all inbound KRPC traffic.

### Modified Capabilities
- `fetch`: Uses direct peer resolution before falling back to iterative walks or trackers.
- `discovery`: Samples attach the reporting node's `SocketAddr` to every emitted `FetchRequest`.

## Impact

- **Fetch Conversion**: Expected to increase from ~0.05% to 5–15%, reducing `empty_peers` failures by >80%.
- **Throughput**: Projected yield increases from ~45 torrents/hr toward 2,500–5,000+ torrents/hr.
- **Memory & Bandwidth**: Eliminates thousands of redundant Kademlia walk queries per minute.
