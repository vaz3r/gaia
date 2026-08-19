## Why

The crawler sustains only ~330 verified torrents/hr against the 10k/hr target. Live measurement isolates the binding constraint: the **routing table is structurally capped**, so the sampler's unique-hash feed collapses to ~9.3k/min (bitmagnet sustains ~131k/min). That cap is the supply wall keeping verified/hr ~30x below target — a table-level defect, not a fetch-level one.

Root cause, confirmed in `routing_table.rs`: the table is pre-partitioned into exactly 160 leading-zero-distance buckets, each capped at `K=80`. Bucket 0 covers **half the keyspace** and saturates at 80 nodes, rejecting ~99% of the nodes that map there. The table therefore stalls at ~2.2k nodes per instance (~9k total) despite processing 6.7M queries/instance — the sampler re-queries the same drained hash sets (97% repeat rate). Bitmagnet's equivalent (`ktable/keyspace.go`) is an **uncapped** B-tree that evicts nodes only on request failure, so its table grows into the 100k+ node range and keeps feeding fresh hash sets.

## What Changes

- **Replace the fixed 160-bucket / K=80 structure** in `routing_table.rs` with a flat, uncapped node store (B-tree keyed by node ID) that keeps every discovered node until it fails repeatedly — matching bitmagnet's `keyspace` semantics.
- **Evict only on failure**: a node is dropped when its `fail_count` reaches the bad threshold; no per-bucket full-rejection, no LRU-of-a-bucket eviction. `max_nodes` becomes a high safety ceiling (~500k), not a per-bucket gate.
- **Retire the leading-zeros bucket-index machinery**: `stale_buckets`/`random_id_in_bucket` bucket-refresh path in `actor.rs` is superseded by the grower's existing continuous whole-table `find_node` cycling; flatten or remove it.
- **Keep the public API surface stable** (`insert`, `closest`, `all_nodes`, `oldest_nodes`, `remove`, `mark_seen`, `mark_failed`, `mark_query`, `len`, `own_id`) so `checked_insert`, persist/save/load (via `all_nodes()`), the sampler, and the grower keep working with minimal call-site churn.
- **Result**: the routing table grows from ~9k to 100k+ nodes, giving the sampler 20-40x more distinct nodes so the unique feed rises toward 200-300k/min.

## Capabilities

### New Capabilities
- `dht-routing-table`: the routing table's node-capacity, eviction, and growth behavior — requirement that the table be uncapped (evict-only-on-failure) so it can scale to 100k+ nodes for distinct-hash breadth.

### Modified Capabilities
_None at the main-spec level._ The routing table is not yet synced to a main spec; this change introduces the capacity/growth requirement as a new capability. (Fetch/conversion levers in the 10k/hr plan are tracked in separate changes and out of scope here.)

## Impact

- **Core code**: `crawler/crates/gaia-dht/src/routing_table.rs` (structure rewrite), `crawler/crates/gaia-dht/src/actor.rs` (bucket-refresh path + any bucket-API call sites), and tests in `routing_table.rs` / `actor.rs`.
- **Behavior**: table can exceed the old 12,800/instance ceiling → higher distinct-node sampling → unique-feed and (at parity conversion) verified/hr gains. No change to fetch dialing, persist format, or the sampler's pick logic in this change.
- **Risk**: a DHT-core rewrite; rebuild + restart required, and 100k+ growth takes longer than 2-min bench windows, so validation needs longer runs. Growth rate must keep pace or the table may still lag behind bitmagnet.
- **Dependencies**: none new; builds on the already-landed grower (`find_node` batch 128 + continuous walkers) and shared rotating `soughtNodeID` sampler.

## Acceptance

- `routing_nodes` exceeds 50k/instance on a sustained bench (well past the old 12,800 cap).
- `unique_per_hr` climbs toward / beyond 300k (from ~558k/hr today's drained state of ~9.3k/min).
- Verified rate trends toward the 10k/hr target given the existing conversion (conversion levers are separate changes).
- All existing routing-table, actor, and sampler tests pass (updated for flat semantics); `cargo clippy --all-targets -- -D warnings` clean.
