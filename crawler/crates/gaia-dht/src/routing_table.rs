#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unchecked_time_subtraction,
    reason = "M175: routing table — node arithmetic and time deltas use post-bootstrap Instants; remaining unchecked-time sites are test fixtures"
)]

//! Kademlia routing table (BEP 5).
//!
//! The routing table maps 160-bit node IDs to socket addresses. It is
//! structured as an **uncapped, flat store keyed by node ID** (bitmagnet's
//! `keyspace` B-tree parity): every discovered node is retained until it fails
//! repeatedly, so the table can grow to 100k+ nodes and sustain the broad
//! distinct-node sampling that drives infohash discovery. A high `max_nodes`
//! safety ceiling bounds memory with LRU eviction; there is no per-distance
//! region cap.

use std::collections::{BTreeMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use gaia_core::Id20;

/// Kademlia `k` parameter: the number of closest nodes returned/used for
/// `find_node`/`get_peers`/lookups. Decoupled from capacity — the table is
/// flat and unbounded, but responses and lookups still operate on this
/// bounded closest-K set.
pub const K: usize = 80;

/// Maximum nodes returned in inbound-query responses (`find_node` /
/// `get_peers` / `get` / `sample_infohashes`). Kept small (BEP 5 semantics) so
/// each answer stays in a single small UDP packet regardless of table size.
pub const RESPONSE_K: usize = 16;

/// Kademlia node liveness classification (BEP 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Node has responded or sent a query within the last 15 minutes.
    Good,
    /// Node has not been active recently but has not failed repeatedly.
    Questionable,
    /// Node has failed 2 or more consecutive queries.
    Bad,
}

/// Age threshold for considering a node "active" (15 minutes).
const ACTIVE_THRESHOLD: Duration = Duration::from_mins(15);

/// Threshold for a node to be considered Bad (dropped/evictable).
const BAD_FAIL_COUNT: u32 = 2;

/// A node in the routing table.
#[derive(Debug, Clone)]
pub struct RoutingNode {
    /// Node's 20-byte Kademlia ID.
    pub id: Id20,
    /// Node's socket address.
    pub addr: SocketAddr,
    /// Timestamp of the last successful interaction.
    pub last_seen: Instant,
    /// Number of consecutive failed queries.
    pub fail_count: u32,
    /// Timestamp of the last response received from this node.
    pub last_response: Option<Instant>,
    /// Timestamp of the last query received from this node.
    pub last_query: Option<Instant>,
}

impl RoutingNode {
    /// Classify this node per Kademlia liveness rules.
    ///
    /// - `Bad`: `fail_count` >= 2
    /// - `Good`: responded or sent a query within the last 15 minutes
    /// - `Questionable`: otherwise
    ///
    /// M175 BUG FIX: previously computed `Instant::now() - ACTIVE_THRESHOLD`,
    /// which panics during the first 15 min of process uptime (Instant
    /// monotonic clock not yet at the threshold). Reformulated to compare
    /// elapsed time forward via `Instant::duration_since`, which saturates at
    /// zero on clock skew. M132-class avoidance — same bug shape as the
    /// `fetch_sub(1)` underflow that shipped in M132.
    #[must_use]
    pub fn status(&self) -> NodeStatus {
        if self.fail_count >= BAD_FAIL_COUNT {
            return NodeStatus::Bad;
        }
        let now = Instant::now();
        let recent = |t: Instant| now.duration_since(t) <= ACTIVE_THRESHOLD;
        let active = self.last_response.is_some_and(recent) || self.last_query.is_some_and(recent);
        if active {
            NodeStatus::Good
        } else {
            NodeStatus::Questionable
        }
    }
}

/// Default maximum number of nodes in the routing table.
///
/// A high safety ceiling (not a per-region gate): bounds memory with LRU
/// eviction while admitting 100k+ nodes, matching bitmagnet's effectively
/// unbounded table so the sampler gets broad distinct-node coverage.
const DEFAULT_MAX_NODES: usize = 500_000;

/// Kademlia routing table.
#[derive(Debug, Clone)]
pub struct RoutingTable {
    own_id: Id20,
    /// Unbounded flat node store keyed by node ID.
    nodes: BTreeMap<Id20, RoutingNode>,
    /// When enabled, tracks IPs to enforce one-node-per-IP (BEP 42).
    ip_set: HashSet<IpAddr>,
    /// Whether to enforce one-node-per-IP restriction.
    restrict_ips: bool,
    /// Safety ceiling on total node count (bounds memory via LRU eviction).
    max_nodes: usize,
}

/// Result of an insert operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertResult {
    /// Node was inserted (new or updated).
    Inserted,
    /// Bucket is full but could be split; caller should try again.
    BucketFull,
    /// Node was not inserted; cannot split further.
    Rejected,
}

impl RoutingTable {
    /// Create a new routing table with the given own node ID.
    #[must_use]
    pub fn new(own_id: Id20) -> Self {
        Self::with_config(own_id, false, DEFAULT_MAX_NODES)
    }

    /// Create a new routing table with IP restriction setting.
    #[must_use]
    pub fn new_with_config(own_id: Id20, restrict_ips: bool) -> Self {
        Self::with_config(own_id, restrict_ips, DEFAULT_MAX_NODES)
    }

    /// Create a new routing table with full configuration.
    #[must_use]
    pub fn with_config(own_id: Id20, restrict_ips: bool, max_nodes: usize) -> Self {
        Self {
            own_id,
            nodes: BTreeMap::new(),
            ip_set: HashSet::new(),
            restrict_ips,
            max_nodes,
        }
    }

    /// Our node ID.
    #[must_use]
    pub fn own_id(&self) -> &Id20 {
        &self.own_id
    }

    /// Total number of nodes in the routing table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the routing table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Number of distinct distance levels currently represented in the table
    /// (populated leading-zero-distance buckets, 0..=159). Mirrors the old
    /// "how many buckets are in use" diagnostic without the fixed 160 ceiling.
    #[must_use]
    pub fn bucket_count(&self) -> usize {
        self.nodes
            .keys()
            .map(|id| self.distance_level(id))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Insert or update a node. Returns `true` if the node was added/updated.
    pub fn insert(&mut self, id: Id20, addr: SocketAddr) -> bool {
        if id == self.own_id {
            return false; // Never insert ourselves
        }
        self.insert_inner(id, addr)
    }

    fn insert_inner(&mut self, id: Id20, addr: SocketAddr) -> bool {
        let ip = addr.ip();

        // BEP 42: check if another node with this IP exists (and it's not the same node)
        if self.restrict_ips && self.ip_set.contains(&ip) && !self.nodes.contains_key(&id) {
            return false; // Different node, same IP — reject
        }

        // Already known — update last_seen and address
        if let Some(existing) = self.nodes.get_mut(&id) {
            let old_ip = existing.addr.ip();
            existing.last_seen = Instant::now();
            existing.addr = addr;
            existing.fail_count = 0;
            // Update IP tracking if address changed
            if self.restrict_ips && old_ip != ip {
                self.ip_set.remove(&old_ip);
                self.ip_set.insert(ip);
            }
            return true;
        }

        // At the safety ceiling: evict one failing node (fail_count > 0,
        // LRU among them), else evict the least-recently-seen node, before
        // admitting the new node. Never reject purely on region fullness.
        if self.nodes.len() >= self.max_nodes {
            let evicted = self.nodes
                .values()
                .enumerate()
                .filter(|(_, n)| n.fail_count > 0)
                .min_by_key(|(_, n)| n.last_seen)
                .or_else(|| {
                    self.nodes
                        .values()
                        .enumerate()
                        .min_by_key(|(_, n)| n.last_seen)
                })
                .map(|(idx, _)| idx);
            if let Some(idx) = evicted {
                // Find the key at the given ordinal position in the BTreeMap.
                let key = self.nodes.iter().nth(idx).map(|(k, _)| *k);
                if let Some(key) = key {
                    if self.restrict_ips {
                        self.ip_set.remove(&self.nodes[&key].addr.ip());
                    }
                    self.nodes.remove(&key);
                }
            }
        }

        // Insert (or re-insert after eviction) the new/updated node.
        if self.restrict_ips {
            self.ip_set.insert(ip);
        }
        self.nodes.insert(
            id,
            RoutingNode {
                id,
                addr,
                last_seen: Instant::now(),
                fail_count: 0,
                last_response: None,
                last_query: None,
            },
        );
        true
    }

    /// Remove a node by ID. Returns `true` if it was present.
    pub fn remove(&mut self, id: &Id20) -> bool {
        if let Some(node) = self.nodes.remove(id) {
            if self.restrict_ips {
                self.ip_set.remove(&node.addr.ip());
            }
            true
        } else {
            false
        }
    }

    /// Mark a node as recently seen.
    pub fn mark_seen(&mut self, id: &Id20) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.last_seen = Instant::now();
            node.fail_count = 0;
        }
    }

    /// Increment a node's fail count.
    pub fn mark_failed(&mut self, id: &Id20) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.fail_count += 1;
        }
    }

    /// Return the `count` closest nodes to `target`, sorted by XOR distance.
    #[must_use]
    pub fn closest(&self, target: &Id20, count: usize) -> Vec<RoutingNode> {
        let mut all: Vec<&RoutingNode> = self.nodes.values().collect();
        all.sort_by_key(|n| n.id.xor_distance(target));
        all.into_iter().take(count).cloned().collect()
    }

    /// Return all nodes in the routing table as (id, addr) pairs.
    #[must_use]
    pub fn all_nodes(&self) -> Vec<(Id20, SocketAddr)> {
        self.nodes
            .values()
            .map(|n| (n.id, n.addr))
            .collect()
    }

    /// Return up to `n` least-recently-seen nodes across the whole table
    /// (bitmagnet's `GetOldestNodes`). `find_node`-ing these refreshes stale
    /// entries so they stay live in the table instead of rotting, and a failed
    /// refresh marks them for eviction — keeping the table at max capacity with
    /// LIVE nodes, which is what drives distinct-node sampling breadth.
    #[must_use]
    pub fn oldest_nodes(&self, n: usize) -> Vec<(Id20, SocketAddr)> {
        let mut all: Vec<&RoutingNode> = self.nodes.values().collect();
        all.sort_by_key(|node| node.last_seen);
        all.truncate(n);
        all.iter().map(|node| (node.id, node.addr)).collect()
    }

    /// Get a reference to a node by ID.
    #[must_use]
    pub fn get(&self, id: &Id20) -> Option<&RoutingNode> {
        self.nodes.get(id)
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_mut(&mut self, id: &Id20) -> Option<&mut RoutingNode> {
        self.nodes.get_mut(id)
    }

    /// Record a successful response from a node, resetting its fail count.
    pub fn mark_response(&mut self, id: &Id20) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.last_response = Some(Instant::now());
            node.fail_count = 0;
        }
    }

    /// Record an incoming query from a node.
    pub fn mark_query(&mut self, id: &Id20) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.last_query = Some(Instant::now());
        }
    }

    /// Mark all nodes in the routing table as Questionable (M97).
    ///
    /// Called when saved-state verification fails — loaded nodes may be stale.
    /// Setting `last_response = None` and `last_query = None` makes nodes
    /// Questionable (never responded, never queried). `fail_count = 0` ensures
    /// they are Questionable rather than Bad.
    pub fn mark_all_questionable(&mut self) {
        for node in self.nodes.values_mut() {
            node.last_response = None;
            node.last_query = None;
            node.fail_count = 0;
        }
    }

    /// Return all nodes whose status is `Questionable`.
    #[must_use]
    pub fn questionable_nodes(&self) -> Vec<(Id20, SocketAddr)> {
        self.nodes
            .values()
            .filter(|n| n.status() == NodeStatus::Questionable)
            .map(|n| (n.id, n.addr))
            .collect()
    }

    // ---- Internal ----

    /// Number of leading matching bits between `own_id` and `id` (0..=159;
    /// only our own ID, never inserted, would reach 160). Used only for the
    /// `bucket_count()` diagnostic; insertion/lookup no longer depend on it.
    fn distance_level(&self, id: &Id20) -> usize {
        let distance = self.own_id.xor_distance(id);
        let mut zeros = 0;
        for &byte in distance.as_bytes() {
            if byte == 0 {
                zeros += 8;
            } else {
                zeros += byte.leading_zeros() as usize;
                break;
            }
        }
        zeros.min(159)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn id(byte: u8) -> Id20 {
        let mut bytes = [0u8; 20];
        bytes[19] = byte;
        Id20(bytes)
    }

    fn addr(port: u16) -> SocketAddr {
        format!("10.0.0.1:{port}").parse().unwrap()
    }

    /// A node ID derived from `n` via SHA-1 so consecutive values spread
    /// uniformly across the whole 160-bit keyspace (every distance level is
    /// reachable), which is what lets the table grow large.
    fn spread_id(n: u64) -> Id20 {
        gaia_core::sha1(&n.to_le_bytes())
    }

    #[test]
    fn table_grows_past_old_ceiling() {
        // The old 160-bucket structure saturated near ~12,800 (and far lower in
        // practice). A flat uncapped table must admit hundreds of thousands.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 500_000);
        for n in 1..20_000u64 {
            let id = spread_id(n);
            rt.insert(id, addr((n % 60_000) as u16));
        }
        assert!(
            rt.len() > 12_800,
            "table should exceed the old 12,800 ceiling, got {}",
            rt.len()
        );
        assert!(
            rt.len() <= 500_000,
            "table must respect the node cap, got {}",
            rt.len()
        );
    }

    #[test]
    fn dense_region_not_saturated() {
        // Many distinct nodes whose IDs share the high-density distance region
        // (leading zero count 0 — half the keyspace) must all be retained rather
        // than rejected after a fixed per-region limit.
        let mut rt = RoutingTable::new(Id20::ZERO);
        for n in 0..1000u64 {
            let mut bytes = [0u8; 20];
            // Top byte >= 0x80 ⇒ leading-zero distance level 0 (half the
            // keyspace), same dense region for every ID.
            bytes[0] = 0x80 | (n & 0x7F) as u8;
            bytes[1] = ((n >> 7) & 0xFF) as u8;
            bytes[2] = ((n >> 15) & 0xFF) as u8;
            bytes[3] = ((n >> 23) & 0xFF) as u8;
            rt.insert(Id20(bytes), addr((n % 60_000) as u16));
        }
        assert_eq!(rt.len(), 1000, "dense region must not reject distinct nodes");
    }

    #[test]
    fn insert_and_retrieve() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        assert!(rt.insert(id(1), addr(1)));
        assert_eq!(rt.len(), 1);
        assert!(rt.get(&id(1)).is_some());
    }

    /// M175 regression: `RoutingNode::status()` previously used
    /// `Instant::now() - ACTIVE_THRESHOLD`, which panics during the first
    /// 15 minutes of process uptime (test process uptime is far below that).
    /// Reformulated using forward `duration_since` so the call is panic-free
    /// regardless of process age.
    #[test]
    fn routing_node_status_does_not_panic_on_fresh_process() {
        let now = Instant::now();
        let node = RoutingNode {
            id: id(1),
            addr: addr(1),
            last_seen: now,
            fail_count: 0,
            last_response: Some(now),
            last_query: None,
        };
        // Just calling status() at all would panic with the pre-M175 form.
        assert_eq!(node.status(), NodeStatus::Good);

        // None timestamps: should not panic and should be Questionable.
        let stale = RoutingNode {
            id: id(2),
            addr: addr(2),
            last_seen: now,
            fail_count: 0,
            last_response: None,
            last_query: None,
        };
        assert_eq!(stale.status(), NodeStatus::Questionable);
    }

    #[test]
    fn insert_self_rejected() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        assert!(!rt.insert(Id20::ZERO, addr(1)));
        assert_eq!(rt.len(), 0);
    }

    #[test]
    fn update_existing_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.insert(id(1), addr(2)); // Update address
        assert_eq!(rt.len(), 1);
        assert_eq!(rt.get(&id(1)).unwrap().addr, addr(2));
    }

    #[test]
    fn remove_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        assert!(rt.remove(&id(1)));
        assert_eq!(rt.len(), 0);
        assert!(!rt.remove(&id(1))); // Already gone
    }

    #[test]
    fn closest_nodes_sorted() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.insert(id(5), addr(5));
        rt.insert(id(3), addr(3));
        rt.insert(id(10), addr(10));

        let closest = rt.closest(&Id20::ZERO, 3);
        assert_eq!(closest.len(), 3);
        // XOR distance from ZERO is just the value itself
        assert_eq!(closest[0].id, id(1));
        assert_eq!(closest[1].id, id(3));
        assert_eq!(closest[2].id, id(5));
    }

    #[test]
    fn closest_fewer_than_count() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        let closest = rt.closest(&Id20::ZERO, 10);
        assert_eq!(closest.len(), 1);
    }

    #[test]
    fn closest_correct_at_scale() {
        // With a large table, closest() must still return the true nearest
        // nodes by XOR distance.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 500_000);
        for n in 1..10_000u64 {
            rt.insert(spread_id(n), addr((n % 60_000) as u16));
        }
        // Target ZERO; the single closest node is the smallest ID present.
        let target = Id20::ZERO;
        let closest = rt.closest(&target, 5);
        assert_eq!(closest.len(), 5);
        // Verify sorted ascending by XOR distance from target.
        let dists: Vec<_> = closest.iter().map(|n| n.id.xor_distance(&target)).collect();
        let mut sorted = dists.clone();
        sorted.sort();
        assert_eq!(dists, sorted);
    }

    #[test]
    fn mark_seen_resets_fail_count() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.mark_failed(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().fail_count, 1);
        rt.mark_seen(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().fail_count, 0);
    }

    #[test]
    fn remove_evicts_ip_slot() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, true);
        let a: SocketAddr = "10.0.0.1:1".parse().unwrap();
        rt.insert(id(1), a);
        rt.remove(&id(1));
        assert_eq!(rt.len(), 0);
    }

    // ── BEP 42 IP restriction tests ────────────────────────────────

    #[test]
    fn restrict_ips_rejects_second_node_same_ip() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, true);
        let ip_addr: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        assert!(rt.insert(id(1), ip_addr));
        // Second node with same IP but different ID — rejected
        let ip_addr2: SocketAddr = "10.0.0.1:6882".parse().unwrap();
        assert!(!rt.insert(id(2), ip_addr2));
        assert_eq!(rt.len(), 1);
    }

    #[test]
    fn restrict_ips_allows_same_node_update() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, true);
        let addr1: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        let addr2: SocketAddr = "10.0.0.1:6882".parse().unwrap();
        assert!(rt.insert(id(1), addr1));
        // Same node ID updating its port — allowed
        assert!(rt.insert(id(1), addr2));
        assert_eq!(rt.len(), 1);
        assert_eq!(rt.get(&id(1)).unwrap().addr, addr2);
    }

    #[test]
    fn no_restrict_ips_allows_multiple_nodes_same_ip() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, false);
        let addr1: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        let addr2: SocketAddr = "10.0.0.1:6882".parse().unwrap();
        assert!(rt.insert(id(1), addr1));
        assert!(rt.insert(id(2), addr2));
        assert_eq!(rt.len(), 2);
    }

    #[test]
    fn restrict_ips_remove_frees_ip_slot() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, true);
        let addr: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        assert!(rt.insert(id(1), addr));
        assert!(rt.remove(&id(1)));
        // IP slot is now free — different node with same IP can insert
        assert!(rt.insert(id(2), addr));
        assert_eq!(rt.len(), 1);
    }

    // ── Liveness / NodeStatus tests ────────────────────────────────

    #[test]
    fn node_status_bad_on_two_failures() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.mark_failed(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Questionable);
        rt.mark_failed(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Bad);
    }

    #[test]
    fn node_status_good_after_mark_response() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        // Freshly inserted nodes have no last_response/last_query, so Questionable
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Questionable);
        rt.mark_response(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Good);
    }

    #[test]
    fn node_status_good_after_mark_query() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.mark_query(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Good);
    }

    #[test]
    fn mark_response_resets_fail_count() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.mark_failed(&id(1));
        rt.mark_failed(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Bad);
        rt.mark_response(&id(1));
        assert_eq!(rt.get(&id(1)).unwrap().fail_count, 0);
        assert_eq!(rt.get(&id(1)).unwrap().status(), NodeStatus::Good);
    }

    #[test]
    fn mark_response_noop_for_unknown_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        // Should not panic when node is not in the table
        rt.mark_response(&id(42));
    }

    #[test]
    fn mark_query_noop_for_unknown_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.mark_query(&id(42));
    }

    #[test]
    fn get_mut_finds_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        let node = rt.get_mut(&id(1));
        assert!(node.is_some());
        // Mutate through the reference
        node.unwrap().fail_count = 99;
        assert_eq!(rt.get(&id(1)).unwrap().fail_count, 99);
    }

    #[test]
    fn get_mut_returns_none_for_missing_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        assert!(rt.get_mut(&id(1)).is_none());
    }

    #[test]
    fn questionable_nodes_filters_correctly() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1)); // Questionable (no activity)
        rt.insert(id(2), addr(2)); // Will be Good
        rt.insert(id(3), addr(3)); // Will be Bad
        rt.mark_response(&id(2));
        rt.mark_failed(&id(3));
        rt.mark_failed(&id(3));

        let q = rt.questionable_nodes();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].0, id(1));
    }

    #[test]
    fn questionable_nodes_empty_when_all_good_or_bad() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(id(1), addr(1));
        rt.insert(id(2), addr(2));
        rt.mark_response(&id(1));
        rt.mark_failed(&id(2));
        rt.mark_failed(&id(2));

        assert!(rt.questionable_nodes().is_empty());
    }

    // ── Node cap tests ─────────────────────────────────────────────

    #[test]
    fn node_cap_rejects_when_no_evictable_node() {
        // With max_nodes=4 and no failing node, a 5th distinct node evicts the
        // LRU (healthy) node rather than being rejected — the flat table always
        // admits new nodes under LRU eviction at the ceiling.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 4);
        for i in 1..=4u8 {
            assert!(rt.insert(id(i), addr(u16::from(i))));
        }
        assert_eq!(rt.len(), 4);
        // 5th insert evicts the least-recently-seen (id 1) and admits id 5.
        assert!(rt.insert(id(5), addr(5)));
        assert_eq!(rt.len(), 4);
        assert!(rt.get(&id(5)).is_some());
        assert!(rt.get(&id(1)).is_none());
    }

    #[test]
    fn node_cap_evicts_failed_node_first() {
        // At the cap, a failing node (fail_count > 0) is evicted before a
        // healthy LRU node, so a failing node can be replaced.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 4);
        for i in 1..=4u8 {
            rt.insert(id(i), addr(u16::from(i)));
        }
        assert_eq!(rt.len(), 4);
        rt.mark_failed(&id(2));
        assert!(rt.insert(id(5), addr(5)));
        assert_eq!(rt.len(), 4);
        assert!(rt.get(&id(5)).is_some());
        assert!(rt.get(&id(2)).is_none(), "failed node evicted first");
    }

    #[test]
    fn node_cap_allows_update() {
        // Updating an existing node must succeed even when at the cap.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 4);
        for i in 1..=4u8 {
            rt.insert(id(i), addr(u16::from(i)));
        }
        assert_eq!(rt.len(), 4);

        // Update existing node — new address, same ID
        assert!(rt.insert(id(2), addr(200)));
        assert_eq!(rt.len(), 4);
        assert_eq!(rt.get(&id(2)).unwrap().addr, addr(200));
    }

    #[test]
    fn table_default_cap_is_high() {
        let rt = RoutingTable::new(Id20::ZERO);
        // Default is a high safety ceiling for unbounded-style growth.
        assert_eq!(rt.max_nodes, DEFAULT_MAX_NODES);
        assert!(rt.max_nodes >= 100_000);
    }

    #[test]
    fn mark_all_questionable_resets_liveness() {
        let own_id = Id20([0x00; 20]);
        let mut rt = RoutingTable::new(own_id);

        // Insert two nodes and mark them as responsive
        let node1 = Id20([0x80; 20]);
        let node2 = Id20([0x40; 20]);
        let addr1: SocketAddr = "192.0.2.1:6881".parse().unwrap();
        let addr2: SocketAddr = "192.0.2.2:6881".parse().unwrap();

        rt.insert(node1, addr1);
        rt.insert(node2, addr2);
        rt.mark_response(&node1);
        rt.mark_response(&node2);

        // Verify nodes are Good
        assert_eq!(rt.get(&node1).unwrap().status(), NodeStatus::Good);
        assert_eq!(rt.get(&node2).unwrap().status(), NodeStatus::Good);

        // Mark all questionable
        rt.mark_all_questionable();

        // Verify nodes are now Questionable
        assert_eq!(rt.get(&node1).unwrap().status(), NodeStatus::Questionable);
        assert_eq!(rt.get(&node2).unwrap().status(), NodeStatus::Questionable);
        assert_eq!(rt.get(&node1).unwrap().fail_count, 0);
        assert_eq!(rt.get(&node2).unwrap().fail_count, 0);
    }
}
