# Tasks

## 1. Phase A — routing table to thousands (D45, D46)

- [x] 1.1 Raise `K` 8 → 80 in `vendor/gaia-dht/src/routing_table.rs`
- [x] 1.2 Add a routing-table growth test (`table_grows_past_old_ceiling`: >1000 nodes with 500k spread)
- [x] 1.3 Replace the last-bucket-only split policy with pre-allocated distance buckets + LRU eviction (far buckets no longer permanently reject at K)
- [x] 1.4 Run the routing-table tests + full gaia-dht suite (247 lib tests pass)

## 2. Phase B — `--scale` knob (D47)

- [x] 2.1 Add `--scale` flag (default 10) in `cli.rs`
- [x] 2.2 Multiply sampler QPS, sampler loops, fetch concurrency, lookup concurrency, channel buffers by scale in `crawler.rs` + `cli.rs`
- [x] 2.3 Wire `--scale` into compose (default 10)

## 3. Phase C — higher sample throughput

- [x] 3.1 Default `sampler_qps` (400→4000) and `sampler_loops` (32→320) scale with `--scale` for a thousands-node table
- [ ] 3.2 Verify sampler keeps up with the larger node pool (no "no ready node" starvation) — measured on deploy

## 4. Phase D — get_peers-first fetch selectivity (D48)

- [x] 4.1 Verified: irontide's DhtLookup only emits non-empty peer batches (`dht_lookup.rs:413`), so `fetch_one` already fails fast as `empty_peers` when a lookup yields no live values — the selectivity requirement is met by the stock lookup semantics
- [x] 4.2 Announce-hint fast path dials the live peer directly, exempt from selectivity (already built in passive-intake phase)

## 5. Integration, deploy, verify

- [ ] 5.1 `cargo test` + `cargo clippy --all-targets -- -D warnings` clean
- [ ] 5.2 Write openspec change package
- [ ] 5.3 Deploy to remote-dev; benchmark vs the ~108/hr baseline
- [ ] 5.4 Tune `--scale` / sampler budgets from measured routing-node growth and torrents/day
