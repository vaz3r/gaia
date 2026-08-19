## Context

See proposal.md — Why. The crawler's routing table is structurally capped at 160 leading-zero-distance buckets × K=80, and high-density bucket 0 (half the keyspace) saturates at 80 nodes and rejects the rest, stalling the table at ~9k total (~2.2k/instance) so the sampler re-queries drained hash sets (97% repeat). Bitmagnet's `ktable/keyspace.go` uses an uncapped B-tree that evicts only on request failure, sustaining 100k+ nodes.

This design covers the Phase 1 structural fix (unbounded table = supply). Phase 2 (conversion / per-IP dial limiter / peer feed) is a separate change and intentionally out of scope.

## Goals / Non-Goals

**Goals**
- Replace the fixed 160-bucket structure with a flat, uncapped node store (evict-only-on-failure) so the table grows 100k+ nodes.
- Preserve the public API surface (`insert`, `closest`, `all_nodes`, `oldest_nodes`, `remove`, `mark_seen`, `mark_failed`, `mark_query`, `len`, `own_id`) and the JSON persist format (built on `all_nodes()`).
- Improve high-density-region acceptance so the table no longer rejects nodes merely because bucket 0 is full.

**Non-Goals**
- No changes to fetch dialing, the per-IP metainfo limiter, `direct_peers` peer feed, or conversion — those are Phase 2 in a separate change.
- No change to the sampler's `pick_target`/sought-target logic.
- No new main-spec capabilities beyond `dht-routing-table` (the routing table is not yet synced to a main spec).

## Decisions

### D1. Flat B-tree keyed by node ID replaces the bucket array
Replace `buckets: Vec<KBucket>` with `BTreeMap<Id20, RoutingNode>` (keyed by node ID) plus the existing `ip_set` for BEP 42. Rationale: `closest()` already collects all nodes and sorts by XOR distance (routing_table.rs:390-394), so bucket indexing provides no incremental benefit for the dominant query — a flat store has the same cost but no capacity ceiling. A `BTreeMap` gives deterministic ordering and O(log n) lookup for `insert`/`mark_*` (vs O(K) linear scan per bucket today).

Alternative considered: keep buckets but raise K and remove per-bucket rejection. Rejected: bucket 0 still owns half the keyspace; even a large K per bucket is a de facto per-region ceiling that reintroduces the exact saturation wall, and `KBucket::find`/`worst_node` stay O(K).

### D2. Evict only on repeated failure; max_nodes = high safety ceiling
- **Insert**: if the ID is new, insert unconditionally. If over `max_nodes` (safety ceiling, raised to ~500k), evict one failing node (`fail_count > 0`, least-recently-seen among them) before inserting; if none failing, evict the least-recently-seen node (bounded cache semantics) to admit the new node — never reject purely on region fullness.
- **Dropping**: `remove()` / eviction trigger only when `fail_count` reaches the bad threshold (2), or when the global safety ceiling forces LRU eviction.
- This mirrors bitmagnet: no per-bucket rejection; the only forced removals are failure-based or a global LRU safety valve.

### D3. Retire the bucket-refresh loop in actor.rs
The `stale_buckets` + `random_id_in_bucket` refresh (actor.rs:2741-2751) exists to keep buckets live via targeted refreshes. The grower in `discovery/mod.rs` already does continuous whole-table `find_node` cycling (batch 128, 4 walkers, 75ms ticks) plus `oldest_nodes` refresh — superseding per-bucket-target refresh.
- Remove the `stale_buckets`/`random_id_in_bucket` refresh block; keep the grower as the freshness mechanism.
- `random_id_in_bucket` is retained only if still prod-coded somewhere; otherwise drop the method and its test.

### D4. Keep the InsertResult enum shape (minimize call-site churn)
`checked_insert` and actor paths consume insert return values. Preserve `InsertResult::{Inserted, BucketFull, Rejected}` as an enum even though `BucketFull` becomes dead/exercised-on-evict; map the new insert to `Inserted` on success and `Rejected` only on the rare safety-ceiling-with-no-evictable case. This keeps `actor.rs` call sites unchanged.

### D5. RESTRICT_IP / BEP 42 preserved
Keep `ip_set` and the one-node-per-IP logic (when `restrict_ips` is on) exactly as-is, operating on the flat store by IP. `--no-restrict-ips` (bench) simply skips it.

## Risks / Trade-offs

- **[Unbounded memory]** A truly unbounded table could grow large. → Keep `max_nodes` (~500k) as an LRU safety ceiling that evicts least-recently-seen nodes only when over the cap; memory bounded, effectively unbounded for crawler scale.
- **[Closest() cost at scale]** `closest()` sorts all nodes per call; at 100k+ nodes this is more expensive per call. → The sampler/grower call it at modest rates; a 100k-element sort is tens of µs in Rust. If it becomes a bottleneck, optimize later (incremental XOR ordering); not needed to prove supply.
- **[Growth may lag bitmagnet]** Removing the cap only helps if the grower actually discovers nodes fast enough. → Keep the grower's batch-128 + 4 walkers; validate `routing_nodes` growth rate during the bench; raise batch/walker count in a follow-up if discovery rate is the new bottleneck.
- **[Persistence size]** A 100k-node JSON persist file is larger. → Still written via `all_nodes()`; acceptable. Could stream/compact later.
- **[DHT-core regression]** Rewrite touches a 1092-line core file + actor call sites. → Update the 11 routing-table + sampler tests for flat semantics; add a regression test that the table exceeds the old 12,800 cap; run `cargo test` + `cargo clippy --all-targets -- -D warnings` before deploy.

## Migration Plan

1. Implement the routing-table rewrite in `gaia-dht` (BTreeMap store, evict-only-on-failure, high `max_nodes`), keeping the public API.
2. Update/retire `stale_buckets` `/random_id_in_bucket` refresh in actor.rs.
3. Update tests for flat semantics; add cap-exceeded regression test.
4. `cargo test` + `cargo clippy`; rebuild.
5. Deploy on fresh DB + redis prefix; monitor `routing_nodes` (expect >50k/instance), `unique_per_hr` (>300k), verified rate trend. Rollback = revert the single commit (table JSON persists independently, no data migration).

## Open Questions

- None blocking. Phase 2 conversion levers (per-IP limiter, peer feed) are scoped to a separate change and do not change the specs/approach here.
