# Phase 3: Active Sybil Weaponization & BEP-51

## Goal Description
Now that the crawler's CPU is no longer starved by DHT floods (Phase 2), we are ready to scale discovery throughput astronomically. 

**The Tradeoff:** As observed post-throttle, dropping 95% of inbound `find_node` queries causes remote nodes to fail their "liveness checks" against us, leading to our eviction from their routing tables. This causes a decay in passive `inbound_get_peers` discovery (dropping from ~3.1M/10min to ~1.2M/10min). 

To completely counteract this and break through the natural limit of a single Node ID, we will implement **Active Sybil Weaponization** combined with **BEP-51 (DHT Infohash Indexing)**. By expanding the Sybil swarm, actively seeding them into remote routing tables, distributing outbound query origins, and introducing BEP-51 `sample_infohashes` support, the crawler will attract substantially more `announce_peer` and `get_peers` messages. 

Crucially, it overhauls the `RoutingTable` to a **Multi-Sybil Routing Table**. The current single Kademlia tree forces distant buckets to drop nodes beyond `K=8`. By maintaining a routing table for each Sybil ID, we map the entire keyspace natively. This ensures that when an infohash lookup starts, we instantly provide starting nodes that are mathematically adjacent to the target, reducing lookup hops from 15 seconds to milliseconds.

## Configurability (The Kill Switch)
The entire Sybil architecture will be fully configurable via the `.env` file using the `CRAW_SYBILS` variable.
*   **ON (`CRAW_SYBILS=128`)**: The crawler spawns 128 virtual identities. The `RoutingTable` provisions 129 internal tables, and outbound queries dynamically rotate through the Sybil IDs to aggressively blanket the network.
*   **OFF (`CRAW_SYBILS=0`)**: The crawler boots as a classic, single-node Kademlia client. The `RoutingTable` allocates exactly 1 internal table, and all queries use your primary `self_id`. This allows you to instantly toggle the weaponization off if you ever need to revert to Phase 2 behavior.

## User Review Required
> [!IMPORTANT]
> The `config.toml` defaults will be updated, but you will need to apply these changes or override them via `.env` in production: `CRAW_SYBILS=128`, `CRAW_SYBIL_BEP42_RATIO=0.125`.
>
> The `RoutingTable` memory footprint will grow from ~1,200 nodes max to ~150,000 nodes max (128 tables * 1,200 nodes). This is negligible for modern RAM (a few megabytes) but is worth noting.

## Proposed Changes

---
### config.rs
#### [MODIFY] src/config.rs
Bump `sybil_count` from 16 to 128, and adjust the `sybil_bep42_ratio` to keep BEP42 IDs capped at ~16 (bound by the single machine IP prefix), while providing 112 random IDs to blanket the keyspace.
```rust
        DhtConfig {
            walker_alpha: 3,
            walker_interval_ms: 250, // Maybe tune down slightly if network is saturated
            sybil_count: 128,                     // Increased from 16
            sybil_bep42_ratio: 16.0 / 128.0,      // Kept BEP42 at ~16
            // ...
```

---
### dht/routing_table.rs
#### [MODIFY] src/dht/routing_table.rs
Rename the existing `RoutingTable` to `SingleRoutingTable`. Create a new `RoutingTable` wrapper that holds a `SingleRoutingTable` for `self_id` and every `sybil_id`. 
*   **`insert`**: Iterates through all internal tables and attempts to insert the node. It returns `true` if *any* table accepted it.
*   **`closest`**: Aggregates the closest nodes from *all* internal tables, sorts them by distance to the target, deduplicates, and truncates to `n`.

```rust
pub struct SingleRoutingTable {
    self_id: NodeId,
    buckets: Vec<VecDeque<NodeInfo>>,
}
// ... (existing RoutingTable impl moved here)

pub struct RoutingTable {
    tables: Vec<SingleRoutingTable>,
}

impl RoutingTable {
    pub fn new(self_id: NodeId, sybils: &[NodeId]) -> Self {
        let mut tables = vec![SingleRoutingTable::new(self_id)];
        for &s in sybils {
            tables.push(SingleRoutingTable::new(s));
        }
        RoutingTable { tables }
    }

    pub fn insert(&mut self, node: NodeInfo) -> bool {
        let mut inserted = false;
        for t in &mut self.tables {
            if t.insert(node) { inserted = true; }
        }
        inserted
    }

    pub fn closest(&self, target: &NodeId, n: usize) -> Vec<NodeInfo> {
        let mut all = Vec::new();
        for t in &self.tables {
            all.extend(t.closest(target, n));
        }
        all.sort_by_key(|node| xor(target, &node.id));
        all.dedup_by(|a, b| a.id == b.id);
        all.truncate(n);
        all
    }
    
    // ... update len() and buckets_used() to aggregate across tables
}
```

---
### router.rs
#### [MODIFY] src/router.rs
1. Update `Router::new` to pass `sybils` to `RoutingTable::new()`.
2. Add a helper to pick a random Sybil ID. This will be used by all outbound query systems to map the DHT from multiple vantage points.
```rust
impl Router {
    // ...
    pub fn random_sybil_id(&self) -> NodeId {
        if self.sybils.is_empty() {
            self.self_id
        } else {
            let idx = rand::random::<usize>() % self.sybils.len();
            self.sybils[idx].0
        }
    }
}
```

---
### verify/peer_source.rs
#### [MODIFY] src/verify/peer_source.rs
1. Modify `source_peers` to use the Sybil ID that is **closest** to the `info_hash` as the sender ID for the queries, rather than `router.self_id` or a random one. This ensures we leverage the Sybil whose routing table will yield the best starting nodes, minimizing lookup hops.
```rust
let sender_id = router.closest_sybil(&info_hash);
// ...
BValue::Bytes(Bytes::copy_from_slice(&sender_id))
```

---
### dht/walker.rs
#### [MODIFY] src/dht/walker.rs
1. Update `bootstrap()` to use `router.random_sybil_id()` instead of `router.self_id` so that even initial handshakes seed Sybils into remote tables.
2. In `pick_target()`, aggressively rotate through all Sybils as the target and sender, forcing remote nodes to track our entire Sybil swarm.

---
### dht/bep51.rs
#### [NEW] src/dht/bep51.rs
Create a new background loop dedicated to BEP-51.
1. Periodically fetch stable nodes from `router.routing_nodes()`.
2. Construct a `sample_infohashes` query with a random `target` and a `router.random_sybil_id()` origin.
3. Parse the `samples` field from the response and send the extracted infohashes directly to `fresh_verify_tx` for immediate verification.

*(This task will be spawned in `main.rs` alongside the walker).*

## Verification Plan

### Automated Tests
Run standard crate tests to ensure syntax and base logic are sound:
`cargo test -p crawler`

### Manual Verification
Monitor the deployment using `health.sh` and production logs:
1. **Routing Table Growth**: The routing table size metric should explode from ~300 to tens of thousands of nodes.
2. **Lookup Hops**: `source_timeout` and `source_deadline_hits` should plummet because lookups complete instantly.
3. **Inbound Announce/Get Peers Rate**: Observe `inbound_get_peers` and `inbound_announce_peer` recovering from the throttle decay and multiplying far beyond their previous ceilings.
4. **BEP-51 Quality**: Track the number of infohashes discovered via BEP-51.
