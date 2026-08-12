# Tasks

## 1. Phase A — node-pool growth (D33, D34)

- [ ] 1.1 `grow_routing` interval 1s → 100ms in `crawler.rs` (per-instance spawn)
- [ ] 1.2 Verify response `nodes` feed-back into routing table (actor side) and that table climbs toward `--max-nodes`
- [ ] 1.3 Raise `PICK_CANDIDATES` sampling spread in `sampler.rs` (64 → configurable, larger default)
- [ ] 1.4 Add per-node productivity deprioritization (0-new-hash → ~5 min backoff) using existing `node_stats`

## 2. Phase B — keyspace node growth

- [x] 2.1 Faster growers (100ms) double as keyspace `get_peers` growth toward random targets (see 1.1)
- [x] 2.2 ~~Announce intake~~ implemented then **reverted**: a `peer_store_hashes()` patch to vendored irontide was built and measured (~1.9% of unique) but cut as not worth the patch cost; `announced_hashes` remains diagnostic-only

## 3. Phase C — cheap dedup / batch triage (D37)

- [x] 3.1 Add a ~10M-entry in-memory bloom filter to short-circuit `scan_blocked` on the sampler hot path
- [x] 3.2 Batch DB triage (~64-hash chunks) for pipeline admission instead of per-hash lookups

## 4. Phase D — fetch tuning + stats (D38)

- [x] 4.1 `FETCH_TIMEOUT` 10s → 5s in `fetch/mod.rs`
- [x] 4.2 Add unique-hash rate (unique/hr) to the stats line
- [x] 4.3 Confirm `concurrency=512` / `lookup_concurrency=256` keep up with the larger stream

## 5. Integration, deploy, verify

- [ ] 5.1 `cargo test` + `cargo clippy --all-targets -- -D warnings` clean
- [ ] 5.2 Deploy to remote-dev; measure torrents/hr, unique/hr, MB/s vs the ~250-400/day baseline
- [ ] 5.3 Tune keyspace-sweep QPS and grower interval from measured node-growth rate
