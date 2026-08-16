# Tasks: Bitmagnet Modernization

## 1. Direct Peer Resolution in gaia-dht & Crawler
- [x] 1.1 Add `direct_get_peers` RPC method to `gaia-dht` actor and `DhtHandle` to send a single direct `get_peers` query to a target `SocketAddr`.
- [x] 1.2 Update `sampler.rs` to ensure every sampled infohash reliably attaches the reporting node's `SocketAddr` in `FetchRequest`.
- [x] 1.3 Update `fetch/mod.rs` to call `direct_get_peers` on the reporting node first before falling back to iterative `get_peers` lookups or trackers.

## 2. Decaying / Generational Bloom Filter
- [x] 2.1 Update `crawler/src/bloom.rs` to implement a generational / decaying bloom filter that evicts old entries over a configurable TTL.
- [x] 2.2 Update `sampler.rs` to remove the permanent insertion of `terminal_dead` hashes into `seen_bloom`.

## 3. Inbound Node Harvesting
- [x] 3.1 Hook into `actor.rs` inbound KRPC query processing to feed sender node IDs and addresses into routing table insertion.
- [x] 3.2 Add test cases in `gaia-dht` validating that inbound messages refresh the routing table.

## 4. Pipeline Decoupling & Concurrency
- [ ] 4.1 Refactor fetch task execution into a decoupled pipeline (Triage -> Direct Peer RPC -> TCP Wire Fetch -> Persistence).
- [x] 4.2 Validate full workspace compilation and unit tests with `cargo test --workspace`.
