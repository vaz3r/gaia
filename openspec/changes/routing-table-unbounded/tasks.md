# Tasks

## 1. Routing table core rewrite (gaia-dht)

- [x] 1.1 Replace `buckets: Vec<KBucket>` with `BTreeMap<Id20, RoutingNode>` in `RoutingTable`; initialize empty in `with_config`; remove `KBucket`, `MAX_BUCKETS`, `bucket_index`, `leading_zeros_160`, `random_id_in_bucket`
- [x] 1.2 Rewrite `insert`/`insert_inner`: insert new IDs unconditionally; on reaching `max_nodes` (raised default), evict a failing node (fail_count>0, LRU among them), else evict least-recently-seen node; still honor `ip_set`/BEP 42 `restrict_ips`
- [x] 1.3 Keep `remove`, `mark_seen`, `mark_failed`, `mark_query`, `mark_all_questionable`, `len`, `is_empty`, `bucket_count` (adapt to flat store; decide keep/drop `bucket_count`)
- [x] 1.4 Keep `closest(target, count)` correct (collect + sort by XOR distance, take count) over the flat store
- [x] 1.5 Keep `all_nodes()` and `oldest_nodes(n)` over the flat store (bitmagnet `GetOldestNodes` parity for the grower)
- [x] 1.6 Raise `with_config` default `max_nodes` to ~500,000 (safety ceiling, not a per-region gate)
- [x] 1.7 Preserve `InsertResult::{Inserted, BucketFull, Rejected}` enum shape; map insert success to Inserted, failure (rare safety-ceiling with no evictable node) to Rejected

## 2. Actor call-site updates

- [x] 2.1 Remove the `stale_buckets` + `random_id_in_bucket` bucket-refresh block in `actor.rs` (grower's continuous whole-table cycling supersedes it)
- [x] 2.2 Verify `checked_insert` and the other `insert`/mark call sites compile unchanged with the flat store API
- [x] 2.3 Confirm `save_routing_table`/`load_routing_table` (via `all_nodes()`) still work; confirm `random_id` seed/bootstrap paths referencing removed bucket APIs are updated

## 3. Tests

- [x] 3.1 Update existing routing-table tests (`routing_table.rs` mod tests) for flat semantics; adjust/remove bucket-specific tests (`random_id_in_bucket`, `bucket_index`-dependent cases)
- [x] 3.2 Add regression test: table exceeds the old 12,800 capacity (insert >13k distinct nodes and assert `len()` grows past it)
- [x] 3.3 Add regression test: dense-region saturation no longer rejects (multiple nodes mapping to high-density distance, all retained)
- [x] 3.4 Add/keep test: evict-only-on-failure (failing node dropped when over ceiling, healthy node not evicted under ceiling)
- [x] 3.5 Update the 11 sampler tests that rely on `pick_target` signatures if the node snapshot type changes; keep them passing
- [x] 3.6 `cargo test` (crawler + gaia-dht) clean
- [ ] 3.7 `cargo clippy --all-targets -- -D warnings` clean (gaia-dht crate clean; crawler bin has pre-existing unrelated clippy failures in committed code — sysmetrics.rs:247, net.rs set_linger, storage dead-code)

## 4. Validate on bench

- [ ] 4.1 Rebuild, restart crawler on fresh DB (`crawler_bench5`) + fresh redis prefix (`dhtbench5`)
- [ ] 4.2 Confirm `routing_nodes` grows past 12,800/instance toward 50k+/instance (well past old cap)
- [ ] 4.3 Confirm `unique_per_hr` climbs toward/over 300k and distinct-node sampling breadth rises (hashes_sampled repeat rate drops below ~50%)
- [ ] 4.4 Track verified rate trend toward 10k/hr given existing conversion; record funnel numbers (`direct_peers_found`, `connect_timeout`) for the Phase-2 conversion handoff
- [ ] 4.5 Open `routing-table-unbounded` spec package / finalize
