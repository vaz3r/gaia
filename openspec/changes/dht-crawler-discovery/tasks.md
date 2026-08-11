# Tasks

## 1. Continuous routing grower (D19)

- [x] 1.1 Add a `grow_routing` loop to `discovery/mod.rs` that issues `get_peers` on random targets on a throttle interval
- [x] 1.2 Replace the one-shot 16-query warmup in `crawler.rs` with a spawned grower task per instance
- [x] 1.3 Grower queries count against the DHT QPS budget and stop on shutdown (cancellation token joined before drain)
- [x] 1.4 Grower interval tuned to ~100ms; instances 1..N bootstrap from instance 0's persisted routing table (`seed_nodes_from_state`) so they don't start empty

## 2. Routing table cap + restrict-ips (D20, D21)

- [x] 2.1 `--max-nodes` default 2048 → 4096 (aggressive 4096 → 8192)
- [x] 2.2 Add `--no-restrict-ips` flag wired to `DhtConfig::restrict_routing_ips`
- [x] 2.3 Build/clippy clean; existing tests pass

## 3. Bootstrap + PM2 (D22)

- [x] 3.1 Expand default bootstrap list (added router.bitcomet.com, bt.offer.bitcomet.com, router.bittorrent.com:6882)
- [x] 3.2 `ecosystem.config.cjs`: `--instances 4` in args
- [x] 3.3 README: grower, `--no-restrict-ips`, 4-instance PM2 default

## 4. Integration, verification

- [x] 4.1 `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release` all green
- [x] 4.2 Restart PM2 (4 instances). Verified: all 4 instances hold real routing tables (100-180 nodes, up from 11-96 without seeding); unique discovery rate rose from 6.9/s to ~20/s; sustained ~80/hr (30-min window) vs 48/hr before
- [x] 4.3 Commit the change set
