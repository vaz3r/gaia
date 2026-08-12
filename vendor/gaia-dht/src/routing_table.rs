#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unchecked_time_subtraction,
    reason = "M175: routing table — node arithmetic and time deltas use post-bootstrap Instants; remaining unchecked-time sites are test fixtures"
)]

//! Kademlia routing table with k-buckets (BEP 5).
//!
//! The routing table maps 160-bit node IDs to socket addresses using
//! a binary-tree of k-buckets. Each bucket holds up to `K` (80) nodes.
//! Buckets are split when the bucket containing our own ID overflows.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use gaia_core::Id20;

/// Maximum nodes per bucket (Kademlia k parameter). Raised from 8 to 80 to
/// match bitmagnet's `nodesK` so a crawler can hold thousands of routing nodes
/// instead of saturating at ~280 — the binding constraint on distinct-node
/// sampling and, therefore, unique-hash discovery.
pub const K: usize = 80;

/// Maximum number of buckets (one per bit of the ID).
const MAX_BUCKETS: usize = 160;

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
        if self.fail_count >= 2 {
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

/// A single k-bucket.
#[derive(Debug, Clone)]
struct KBucket {
    nodes: Vec<RoutingNode>,
}

impl KBucket {
    fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(K),
        }
    }

    fn is_full(&self) -> bool {
        self.nodes.len() >= K
    }

    fn find(&self, id: &Id20) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == *id)
    }

    /// Return the node with the highest fail count, or the oldest if tied.
    fn worst_node(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.fail_count
                    .cmp(&b.fail_count)
                    .then(b.last_seen.cmp(&a.last_seen))
            })
            .map(|(i, _)| i)
    }

    /// Return the least-recently-seen node (for LRU replacement when a bucket
    /// is full of healthy nodes).
    fn oldest_node(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.last_seen.cmp(&b.last_seen))
            .map(|(i, _)| i)
    }
}

/// Default maximum number of nodes in the routing table.
/// Matches rqbit's default and prevents unbounded growth from adversarial injection.
const DEFAULT_MAX_NODES: usize = 512;

/// Kademlia routing table.
#[derive(Debug, Clone)]
pub struct RoutingTable {
    own_id: Id20,
    buckets: Vec<KBucket>,
    /// When enabled, tracks IPs to enforce one-node-per-IP (BEP 42).
    ip_set: HashSet<IpAddr>,
    /// Whether to enforce one-node-per-IP restriction.
    restrict_ips: bool,
    /// Maximum number of nodes allowed in the routing table.
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
        // Pre-allocate every distance bucket (0..MAX_BUCKETS-1) keyed by the
        // exact leading-zeros distance from our own ID. Unlike a lazy
        // last-bucket-split table — where far buckets fill to K=80 and then
        // permanently reject — each level gets its own bucket, so the table
        // can hold up to K × MAX_BUCKETS nodes across the whole keyspace
        // (bitmagnet's `nodesK=80` splittable-trie equivalent).
        let buckets = (0..MAX_BUCKETS).map(|_| KBucket::new()).collect();
        Self {
            own_id,
            buckets,
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
        self.buckets.iter().map(|b| b.nodes.len()).sum()
    }

    /// Whether the routing table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of buckets.
    #[must_use]
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
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
        if self.restrict_ips && self.ip_set.contains(&ip) {
            let bucket_idx = self.bucket_index(&id);
            if self.buckets[bucket_idx].find(&id).is_none() {
                return false; // Different node, same IP — reject
            }
        }

        let bucket_idx = self.bucket_index(&id);

        // Already known — update last_seen and address
        if let Some(pos) = self.buckets[bucket_idx].find(&id) {
            let old_ip = self.buckets[bucket_idx].nodes[pos].addr.ip();
            self.buckets[bucket_idx].nodes[pos].last_seen = Instant::now();
            self.buckets[bucket_idx].nodes[pos].addr = addr;
            self.buckets[bucket_idx].nodes[pos].fail_count = 0;
            // Update IP tracking if address changed
            if self.restrict_ips && old_ip != ip {
                self.ip_set.remove(&old_ip);
                self.ip_set.insert(ip);
            }
            return true;
        }

        // Global node cap — when at limit, only allow insertion by evicting a
        // bad node (fail_count > 0) from this bucket. This keeps the table
        // bounded while still allowing fresh nodes to replace stale ones.
        let at_cap = self.len() >= self.max_nodes;
        if at_cap {
            if let Some(worst_idx) = self.buckets[bucket_idx].worst_node()
                && self.buckets[bucket_idx].nodes[worst_idx].fail_count > 0
            {
                if self.restrict_ips {
                    self.ip_set
                        .remove(&self.buckets[bucket_idx].nodes[worst_idx].addr.ip());
                }
                self.buckets[bucket_idx].nodes[worst_idx] = RoutingNode {
                    id,
                    addr,
                    last_seen: Instant::now(),
                    fail_count: 0,
                    last_response: None,
                    last_query: None,
                };
                if self.restrict_ips {
                    self.ip_set.insert(ip);
                }
                return true;
            }
            // No bad nodes to evict — reject
            return false;
        }

        // Room in bucket
        if !self.buckets[bucket_idx].is_full() {
            self.buckets[bucket_idx].nodes.push(RoutingNode {
                id,
                addr,
                last_seen: Instant::now(),
                fail_count: 0,
                last_response: None,
                last_query: None,
            });
            if self.restrict_ips {
                self.ip_set.insert(ip);
            }
            return true;
        }

        // Bucket full — try to evict a failed node
        if let Some(worst_idx) = self.buckets[bucket_idx].worst_node()
            && self.buckets[bucket_idx].nodes[worst_idx].fail_count > 0
        {
            // Remove old node's IP from tracking (gap fix #7)
            if self.restrict_ips {
                self.ip_set
                    .remove(&self.buckets[bucket_idx].nodes[worst_idx].addr.ip());
            }
            self.buckets[bucket_idx].nodes[worst_idx] = RoutingNode {
                id,
                addr,
                last_seen: Instant::now(),
                fail_count: 0,
                last_response: None,
                last_query: None,
            };
            if self.restrict_ips {
                self.ip_set.insert(ip);
            }
            return true;
        }

        // Bucket full, all nodes good — evict the least-recently-seen node so
        // the table keeps the freshest K nodes per distance level. (With
        // pre-allocated buckets there is no catch-all to split; a crawler
        // wants maximum freshness and diversity, so LRU replacement beats
        // rejecting new nodes outright.)
        if let Some(oldest_idx) = self.buckets[bucket_idx].oldest_node() {
            if self.restrict_ips {
                self.ip_set
                    .remove(&self.buckets[bucket_idx].nodes[oldest_idx].addr.ip());
            }
            self.buckets[bucket_idx].nodes[oldest_idx] = RoutingNode {
                id,
                addr,
                last_seen: Instant::now(),
                fail_count: 0,
                last_response: None,
                last_query: None,
            };
            if self.restrict_ips {
                self.ip_set.insert(ip);
            }
            return true;
        }

        false
    }

    /// Remove a node by ID. Returns `true` if it was present.
    pub fn remove(&mut self, id: &Id20) -> bool {
        let bucket_idx = self.bucket_index(id);
        let bucket = &mut self.buckets[bucket_idx];
        if let Some(pos) = bucket.find(id) {
            if self.restrict_ips {
                self.ip_set.remove(&bucket.nodes[pos].addr.ip());
            }
            bucket.nodes.remove(pos);
            true
        } else {
            false
        }
    }

    /// Mark a node as recently seen.
    pub fn mark_seen(&mut self, id: &Id20) {
        let bucket_idx = self.bucket_index(id);
        if let Some(pos) = self.buckets[bucket_idx].find(id) {
            self.buckets[bucket_idx].nodes[pos].last_seen = Instant::now();
            self.buckets[bucket_idx].nodes[pos].fail_count = 0;
        }
    }

    /// Increment a node's fail count.
    pub fn mark_failed(&mut self, id: &Id20) {
        let bucket_idx = self.bucket_index(id);
        if let Some(pos) = self.buckets[bucket_idx].find(id) {
            self.buckets[bucket_idx].nodes[pos].fail_count += 1;
        }
    }

    /// Return the `count` closest nodes to `target`, sorted by XOR distance.
    #[must_use]
    pub fn closest(&self, target: &Id20, count: usize) -> Vec<RoutingNode> {
        let mut all: Vec<&RoutingNode> = self.buckets.iter().flat_map(|b| &b.nodes).collect();
        all.sort_by_key(|n| n.id.xor_distance(target));
        all.into_iter().take(count).cloned().collect()
    }

    /// Return bucket indices that haven't been refreshed recently.
    ///
    /// M175 BUG FIX: same shape as `RoutingNode::status` — `Instant::now() - max_age`
    /// panics if `max_age` exceeds process uptime. Compares elapsed time forward
    /// instead.
    #[must_use]
    pub fn stale_buckets(&self, max_age: std::time::Duration) -> Vec<usize> {
        let now = Instant::now();
        self.buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.nodes.is_empty()
                    || b.nodes
                        .iter()
                        .all(|n| now.duration_since(n.last_seen) > max_age)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Generate a random ID that falls within the given bucket index.
    /// Useful for refreshing stale buckets.
    #[must_use]
    pub fn random_id_in_bucket(&self, bucket_idx: usize) -> Id20 {
        let mut id = self.own_id;
        // Flip the bit at position `bucket_idx` to land in that bucket's range.
        // Buckets cover distances where the first differing bit is at position bucket_idx.
        if bucket_idx < MAX_BUCKETS {
            let byte_idx = bucket_idx / 8;
            let bit_idx = 7 - (bucket_idx % 8);
            id.0[byte_idx] ^= 1 << bit_idx;
        }
        id
    }

    /// Return all nodes in the routing table as (id, addr) pairs.
    #[must_use]
    pub fn all_nodes(&self) -> Vec<(Id20, SocketAddr)> {
        self.buckets
            .iter()
            .flat_map(|b| b.nodes.iter().map(|n| (n.id, n.addr)))
            .collect()
    }

    /// Get a reference to a node by ID.
    #[must_use]
    pub fn get(&self, id: &Id20) -> Option<&RoutingNode> {
        let bucket_idx = self.bucket_index(id);
        self.buckets[bucket_idx]
            .find(id)
            .map(|pos| &self.buckets[bucket_idx].nodes[pos])
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_mut(&mut self, id: &Id20) -> Option<&mut RoutingNode> {
        let bucket_idx = self.bucket_index(id);
        let pos = self.buckets[bucket_idx].find(id)?;
        Some(&mut self.buckets[bucket_idx].nodes[pos])
    }

    /// Record a successful response from a node, resetting its fail count.
    pub fn mark_response(&mut self, id: &Id20) {
        let bucket_idx = self.bucket_index(id);
        if let Some(pos) = self.buckets[bucket_idx].find(id) {
            self.buckets[bucket_idx].nodes[pos].last_response = Some(Instant::now());
            self.buckets[bucket_idx].nodes[pos].fail_count = 0;
        }
    }

    /// Record an incoming query from a node.
    pub fn mark_query(&mut self, id: &Id20) {
        let bucket_idx = self.bucket_index(id);
        if let Some(pos) = self.buckets[bucket_idx].find(id) {
            self.buckets[bucket_idx].nodes[pos].last_query = Some(Instant::now());
        }
    }

    /// Mark all nodes in the routing table as Questionable (M97).
    ///
    /// Called when saved-state verification fails — loaded nodes may be stale.
    /// Setting `last_response = None` and `last_query = None` makes nodes
    /// Questionable (never responded, never queried). `fail_count = 0` ensures
    /// they are Questionable rather than Bad.
    pub fn mark_all_questionable(&mut self) {
        for bucket in &mut self.buckets {
            for node in &mut bucket.nodes {
                node.last_response = None;
                node.last_query = None;
                node.fail_count = 0;
            }
        }
    }

    /// Return all nodes whose status is `Questionable`.
    #[must_use]
    pub fn questionable_nodes(&self) -> Vec<(Id20, SocketAddr)> {
        self.buckets
            .iter()
            .flat_map(|b| {
                b.nodes
                    .iter()
                    .filter(|n| n.status() == NodeStatus::Questionable)
                    .map(|n| (n.id, n.addr))
            })
            .collect()
    }

    // ---- Internal ----

    /// Determine which bucket a node ID belongs to.
    ///
    /// The bucket index is the number of leading matching bits between
    /// `own_id` and `id`, clamped to the last bucket. Every bucket is
    /// pre-allocated, so a node maps to its exact distance level.
    fn bucket_index(&self, id: &Id20) -> usize {
        let distance = self.own_id.xor_distance(id);
        let leading_zeros = leading_zeros_160(&distance);
        // Clamp to the last bucket (distance 0..MAX_BUCKETS-1 are exact;
        // only our own ID, which is never inserted, would reach 160).
        leading_zeros.min(MAX_BUCKETS - 1)
    }
}

/// Count leading zero bits in a 160-bit (20-byte) value.
fn leading_zeros_160(id: &Id20) -> usize {
    let mut zeros = 0;
    for &byte in id.as_bytes() {
        if byte == 0 {
            zeros += 8;
        } else {
            zeros += byte.leading_zeros() as usize;
            break;
        }
    }
    zeros
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

    /// IDs that share the same distance bucket: bytes 18-19 set so every value
    /// has the same leading-zero count from the origin (0x00..0x00 0x80.. → lz
    /// 144). With pre-allocated buckets this is what fills one bucket to K.
    fn same_bucket_id(n: u8) -> Id20 {
        let mut bytes = [0u8; 20];
        bytes[18] = 0x01;
        bytes[19] = 0x80 | (n & 0x7F);
        Id20(bytes)
    }

    /// A node ID derived from `n` via SHA-1 so consecutive values spread
    /// uniformly across the whole 160-bit keyspace (every distance level is
    /// reachable), which is what lets the table grow into many buckets.
    fn spread_id(n: u64) -> Id20 {
        gaia_core::sha1(&n.to_le_bytes())
    }

    #[test]
    fn table_grows_past_old_ceiling() {
        // K=8 saturated around ~280 nodes. With K=80 each pre-allocated
        // distance level holds up to 80 nodes, so the table grows toward
        // K × log2(N) — thousands as the observed node population grows. 500k
        // uniformly-spread nodes must yield well past the old 280 ceiling.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 8192);
        for n in 1..500_000u64 {
            let id = spread_id(n);
            rt.insert(id, addr((n % 60_000) as u16));
        }
        assert!(
            rt.len() > 1000,
            "table should exceed the old ~280 ceiling, got {}",
            rt.len()
        );
        assert!(
            rt.len() <= 8192,
            "table must respect the node cap, got {}",
            rt.len()
        );
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

    /// M175 regression: `RoutingTable::stale_buckets` previously used
    /// `Instant::now() - max_age` and panicked when `max_age` exceeded
    /// process uptime. Should now compare elapsed time forward.
    #[test]
    fn stale_buckets_does_not_panic_on_large_max_age() {
        let rt = RoutingTable::new(Id20::ZERO);
        // 24-hour max_age — far exceeds any test process uptime.
        let _ = rt.stale_buckets(std::time::Duration::from_hours(24));
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
    fn bucket_splitting() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        // Insert K+1 nodes — should trigger split since bucket holds K=80.
        for i in 1..=(K as u16 + 1) {
            rt.insert(id((i % 256) as u8), addr(i));
        }
        assert!(rt.bucket_count() > 1);
        assert_eq!(rt.len(), K + 1);
    }

    #[test]
    fn evict_failed_node() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        // Fill bucket to capacity
        for i in 1..=K as u8 {
            rt.insert(id(i), addr(u16::from(i)));
        }
        assert_eq!(rt.len(), K);

        // Mark a node as failed
        rt.mark_failed(&id(1));
        rt.mark_failed(&id(1));

        // This should evict the failed node
        let new_id = id(100);
        assert!(rt.insert(new_id, addr(100)));
        assert!(rt.get(&new_id).is_some());
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
    fn stale_buckets_detection() {
        let mut rt = RoutingTable::new(Id20::ZERO);
        // Empty routing table — every pre-allocated bucket is stale
        let stale = rt.stale_buckets(std::time::Duration::from_mins(15));
        assert_eq!(stale.len(), rt.bucket_count());

        // Insert a node — its bucket should not be stale
        rt.insert(id(1), addr(1));
        let stale = rt.stale_buckets(std::time::Duration::from_mins(15));
        assert!(stale.len() == rt.bucket_count() - 1, "one bucket refreshed");
    }

    #[test]
    fn leading_zeros_correct() {
        assert_eq!(leading_zeros_160(&Id20::ZERO), 160);
        assert_eq!(leading_zeros_160(&id(1)), 159);
        assert_eq!(leading_zeros_160(&id(128)), 152);
        let mut full = [0xFFu8; 20];
        assert_eq!(leading_zeros_160(&Id20(full)), 0);
        full[0] = 0x01;
        full[1..].fill(0);
        assert_eq!(leading_zeros_160(&Id20(full)), 7);
    }

    #[test]
    fn random_id_in_bucket_differs() {
        let rt = RoutingTable::new(Id20::ZERO);
        let rand_id = rt.random_id_in_bucket(0);
        assert_ne!(rand_id, Id20::ZERO);
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

    #[test]
    fn worst_node_evicts_oldest_on_tied_fail_counts() {
        // Build a bucket manually to control insertion order and last_seen values
        let mut bucket = KBucket::new();
        let now = Instant::now();
        // Node A: inserted earlier (older last_seen), fail_count=1
        bucket.nodes.push(RoutingNode {
            id: id(1),
            addr: addr(1),
            last_seen: now - std::time::Duration::from_secs(100),
            fail_count: 1,
            last_response: None,
            last_query: None,
        });
        // Node B: inserted more recently (newer last_seen), fail_count=1
        bucket.nodes.push(RoutingNode {
            id: id(2),
            addr: addr(2),
            last_seen: now - std::time::Duration::from_secs(10),
            fail_count: 1,
            last_response: None,
            last_query: None,
        });
        // worst_node should return index 0 (the oldest node)
        let worst = bucket.worst_node().unwrap();
        assert_eq!(
            bucket.nodes[worst].id,
            id(1),
            "oldest node should be evicted on tied fail counts"
        );
    }

    #[test]
    fn worst_node_prefers_highest_fail_count() {
        let mut bucket = KBucket::new();
        let now = Instant::now();
        bucket.nodes.push(RoutingNode {
            id: id(1),
            addr: addr(1),
            last_seen: now,
            fail_count: 3,
            last_response: None,
            last_query: None,
        });
        bucket.nodes.push(RoutingNode {
            id: id(2),
            addr: addr(2),
            last_seen: now - std::time::Duration::from_secs(1000),
            fail_count: 1,
            last_response: None,
            last_query: None,
        });
        let worst = bucket.worst_node().unwrap();
        assert_eq!(
            bucket.nodes[worst].id,
            id(1),
            "highest fail_count should win regardless of age"
        );
    }

    #[test]
    fn restrict_ips_eviction_frees_ip_slot() {
        let mut rt = RoutingTable::new_with_config(Id20::ZERO, true);
        // Fill a single distance bucket to capacity, each with different IP
        for i in 1..=K as u8 {
            let a: SocketAddr = format!("10.0.0.{i}:6881").parse().unwrap();
            rt.insert(same_bucket_id(i), a);
        }
        assert_eq!(rt.len(), K);

        // Mark a node as failed
        rt.mark_failed(&same_bucket_id(1));
        rt.mark_failed(&same_bucket_id(1));

        // New node with a different IP should evict the failed node
        let new_addr: SocketAddr = "10.0.0.100:6881".parse().unwrap();
        assert!(rt.insert(same_bucket_id(K as u8 + 1), new_addr));
        assert_eq!(rt.len(), K);
        // The old IP (10.0.0.1) should be freed
        let old_addr: SocketAddr = "10.0.0.1:6882".parse().unwrap();
        assert!(rt.insert(same_bucket_id(K as u8 + 2), old_addr));
    }

    // ── Node cap tests ─────────────────────────────────────────────

    #[test]
    fn routing_table_node_cap_rejects_at_limit() {
        // With max_nodes=4, inserting a 5th node should fail when no bad nodes
        // exist in the target bucket.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 4);
        for i in 1..=4u8 {
            assert!(
                rt.insert(id(i), addr(u16::from(i))),
                "insert {i} should succeed"
            );
        }
        assert_eq!(rt.len(), 4);
        // 5th insert — all nodes are good, should be rejected
        assert!(!rt.insert(id(5), addr(5)));
        assert_eq!(rt.len(), 4);
    }

    #[test]
    fn routing_table_node_cap_allows_eviction() {
        // At the cap, a new node can still be inserted if a bad node (fail_count > 0)
        // exists in the target bucket and can be evicted.
        let mut rt = RoutingTable::with_config(Id20::ZERO, false, 4);
        // Use same-bucket IDs so all four nodes contend for one bucket.
        for i in 1..=4u8 {
            rt.insert(same_bucket_id(i), addr(u16::from(i)));
        }
        assert_eq!(rt.len(), 4);

        // Mark node 1 as failed so it can be evicted
        rt.mark_failed(&same_bucket_id(1));

        // Insert at cap succeeds by evicting the failed node
        assert!(rt.insert(same_bucket_id(5), addr(5)));
        assert_eq!(rt.len(), 4);
        assert!(rt.get(&same_bucket_id(5)).is_some());
        assert!(rt.get(&same_bucket_id(1)).is_none());
    }

    #[test]
    fn routing_table_node_cap_allows_update() {
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
    fn routing_table_default_cap_512() {
        let rt = RoutingTable::new(Id20::ZERO);
        // The default max_nodes should be 512
        assert_eq!(rt.max_nodes, DEFAULT_MAX_NODES);
        assert_eq!(rt.max_nodes, 512);
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
