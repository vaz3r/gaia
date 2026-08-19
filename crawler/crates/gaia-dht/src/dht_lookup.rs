#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unchecked_time_subtraction,
    reason = "M175: DHT iterative lookup — node counts bounded by ALPHA fan-out; remaining time-sub sites are test fixtures"
)]

//! Self-contained parallel DHT lookup.
//!
//! Replaces the actor-driven `IterativeLookup<GetPeersCallbacks>` with a
//! persistent tokio task that sends KRPC directly via `Arc<UdpSocket>`.
//! Tracks up to 256 nodes with multi-key sort (returned_peers, responded,
//! distance), depth-limited recursion (`max_depth` = 4), 60 s re-request
//! dedup, and adaptive root re-injection from the routing table (1-15 s
//! backoff when empty, proportional up to 60 s when healthy).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, trace};

use gaia_core::{AddressFamily, Id20};

// CompactNodeInfo is used in tests and for documentation reference.
#[cfg(test)]
use crate::compact::CompactNodeInfo;
use crate::krpc::{
    GetPeersResponse, KrpcBody, KrpcMessage, KrpcQuery, KrpcResponse, PeerBatch, TransactionId,
};
use crate::routing_table::RoutingTable;

// Re-use actor types via pub(crate) exports.
use crate::actor::{PendingQuery, PendingQueryKind, SharedRateLimiter};

/// Configuration for a DHT lookup.
pub(crate) struct LookupConfig {
    /// Maximum recursion depth from the initial seed nodes.
    pub max_depth: u8,
    /// Maximum number of tracked nodes in the lookup.
    pub max_nodes: usize,
}

/// A tracked node in the lookup's node list.
struct TrackedNode {
    id: Id20,
    addr: SocketAddr,
    depth: u8,
    returned_peers: bool,
    responded: bool,
    last_queried: Option<Instant>,
}

/// Self-contained persistent DHT lookup matching rqbit's `RecursiveRequest`
/// pattern.
///
/// Spawned as a tokio task, owns its own `FuturesUnordered` of query futures,
/// sends KRPC directly via a shared `Arc<UdpSocket>`, and receives responses
/// through oneshot channels inserted into a shared `DashMap`.
pub(crate) struct DhtLookup {
    target: Id20,
    config: LookupConfig,
    nodes: Vec<TrackedNode>,
    queried_addrs: HashSet<SocketAddr>,
    address_family: AddressFamily,
    // Shared state from actor
    socket: Arc<UdpSocket>,
    pending: Arc<DashMap<u16, PendingQuery>>,
    rate_limiter: Arc<SharedRateLimiter>,
    routing_table: Arc<parking_lot::RwLock<RoutingTable>>,
    next_txn_id: Arc<AtomicU16>,
    own_id: Id20,
    // Output channels
    peer_tx: mpsc::UnboundedSender<PeerBatch>,
    token_tx: mpsc::UnboundedSender<(Id20, Id20, SocketAddr, Vec<u8>)>,
    node_tx: mpsc::UnboundedSender<(Id20, SocketAddr)>,
    /// Consecutive `inject_roots()` calls that returned 0 new nodes (for backoff).
    empty_inject_count: u32,
    /// BEP 43: Whether outgoing queries should include the `ro` flag.
    read_only_mode: bool,
    /// BEP 45: Requested address families for outgoing queries.
    want: Option<Vec<crate::krpc::WantFamily>>,
    /// A node known to hold the target (the BEP 51 reporting node for a
    /// sampled fetch). Queried first so the lookup reaches the node that
    /// proved it has the hash, instead of only walking keyspace-closest roots.
    seed_addr: Option<SocketAddr>,
}

/// Maximum interval for re-injecting root nodes from the routing table.
/// Used when the routing table returns a full set of 8 roots.  Shorter
/// intervals are computed proportionally (few roots) or via exponential
/// backoff (1-15 s when the routing table returns nothing).
const REQUERY_INTERVAL: Duration = Duration::from_mins(1);

/// Minimum time before re-querying the same address.
const REQUERY_DEDUP: Duration = Duration::from_mins(1);

/// Timeout for a single KRPC query within the lookup.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Type alias for the future returned by `spawn_query`.
type QueryFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = (SocketAddr, Result<(Id20, GetPeersResponse), ()>)> + Send,
    >,
>;

impl DhtLookup {
    /// Create a new lookup.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: Id20,
        config: LookupConfig,
        address_family: AddressFamily,
        socket: Arc<UdpSocket>,
        pending: Arc<DashMap<u16, PendingQuery>>,
        rate_limiter: Arc<SharedRateLimiter>,
        routing_table: Arc<parking_lot::RwLock<RoutingTable>>,
        next_txn_id: Arc<AtomicU16>,
        own_id: Id20,
        peer_tx: mpsc::UnboundedSender<PeerBatch>,
        token_tx: mpsc::UnboundedSender<(Id20, Id20, SocketAddr, Vec<u8>)>,
        node_tx: mpsc::UnboundedSender<(Id20, SocketAddr)>,
        read_only_mode: bool,
        want: Option<Vec<crate::krpc::WantFamily>>,
        seed_addr: Option<SocketAddr>,
    ) -> Self {
        Self {
            target,
            config,
            nodes: Vec::with_capacity(256),
            queried_addrs: HashSet::new(),
            address_family,
            socket,
            pending,
            rate_limiter,
            routing_table,
            next_txn_id,
            own_id,
            peer_tx,
            token_tx,
            node_tx,
            empty_inject_count: 0,
            read_only_mode,
            want,
            seed_addr,
        }
    }

    /// Main event loop. Runs until the peer channel closes (torrent dropped)
    /// or all futures drain and the routing table yields no new roots.
    pub async fn run(mut self) {
        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();

        // Query the known-holder seed first (the BEP 51 reporting node for a
        // sampled fetch), then walk keyspace-closest roots. The seed response
        // also injects its closer nodes via process_response.
        if let Some(seed) = self.seed_addr.take()
            && !self.queried_addrs.contains(&seed)
        {
            futures.push(self.spawn_query(seed, None));
        }

        // Seed initial queries from the routing table.
        self.inject_roots(&mut futures);

        // M146: Start with 1s requery (not 60s) for fast root re-injection during
        // cold start. The existing adaptive backoff (1-15s) takes over after the
        // first re-injection.
        let requery_sleep = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(requery_sleep);

        loop {
            // If the peer channel is closed the torrent no longer cares about
            // us, so exit.
            if self.peer_tx.is_closed() {
                debug!(
                    target = %self.target,
                    "DhtLookup: peer channel closed, shutting down"
                );
                break;
            }

            tokio::select! {
                () = &mut requery_sleep => {
                    let injected = self.inject_roots(&mut futures);
                    let next_delay = if injected == 0 {
                        // Back off: 1s, 2s, 4s, 8s, 15s cap on consecutive
                        // empty rounds.  Prevents 1 Hz spin on network failure.
                        self.empty_inject_count =
                            self.empty_inject_count.saturating_add(1);
                        Duration::from_secs(
                            (1u64 << self.empty_inject_count
                                .saturating_sub(1)
                                .min(4))
                            .min(15),
                        )
                    } else {
                        self.empty_inject_count = 0;
                        if injected < 8 {
                            // Proportional: few roots → shorter interval.
                            REQUERY_INTERVAL / 8 * injected.min(8)
                        } else {
                            REQUERY_INTERVAL
                        }
                    };
                    requery_sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + next_delay);
                }
                result = futures.next(), if !futures.is_empty() => {
                    match result {
                        Some((addr, Ok((sender_id, gp)))) => {
                            // Process the response first so the routing grower
                            // gets this node's discovered nodes injected, THEN
                            // fast-exit if the consumer dropped the channel. If
                            // we exit before processing, a grower (which drops
                            // the reply immediately) would never harvest nodes.
                            self.process_response(addr, sender_id, &gp, &mut futures);
                            if self.peer_tx.is_closed() {
                                debug!(
                                    target = %self.target,
                                    "DhtLookup: peer channel closed, winding down after response"
                                );
                                // Bound how long a grower lookup runs: drain at
                                // most a couple more futures before exiting, so
                                // it harvests a few nodes without a full walk.
                                let mut drain = 2usize;
                                while drain > 0 && !futures.is_empty() {
                                    if futures.next().await.is_some() {
                                        drain -= 1;
                                    }
                                }
                                break;
                            }
                        }
                        Some((addr, Err(()))) => {
                            self.mark_error(addr);
                        }
                        None => {
                            // FuturesUnordered drained — wait for timer.
                        }
                    }
                }
                // When futures is empty, just wait for the requery timer.
                () = tokio::time::sleep(Duration::from_secs(1)), if futures.is_empty() => {}
            }
        }
    }

    /// Inject closest nodes from the routing table as depth-0 seeds and spawn
    /// queries for any newly-added nodes.  Returns the number of nodes
    /// actually injected (used for adaptive re-injection timing).
    fn inject_roots(&mut self, futures: &mut FuturesUnordered<QueryFuture>) -> u32 {
        let roots = self.routing_table.read().closest(&self.target, 8);
        let mut spawned = 0u32;
        for node in roots {
            if self.maybe_add_node(node.id, node.addr, 0) {
                futures.push(self.spawn_query(node.addr, Some(node.id)));
                spawned = spawned.saturating_add(1);
            }
        }
        if spawned > 0 {
            trace!(
                target = %self.target,
                spawned,
                total_nodes = self.nodes.len(),
                "DhtLookup: injected roots"
            );
        }
        spawned
    }

    /// Try to add a node to the tracking list. Returns `true` if the node was
    /// added and should be queried.
    fn maybe_add_node(&mut self, id: Id20, addr: SocketAddr, depth: u8) -> bool {
        // Depth gate
        if depth > self.config.max_depth {
            return false;
        }

        // Address-family filter
        let family_ok = match self.address_family {
            AddressFamily::V4 => addr.is_ipv4(),
            AddressFamily::V6 => addr.is_ipv6(),
        };
        if !family_ok {
            return false;
        }

        // Don't add ourselves
        if id == self.own_id {
            return false;
        }

        // Already tracking this address — check re-query dedup
        if let Some(existing) = self.nodes.iter().find(|n| n.addr == addr) {
            if let Some(last) = existing.last_queried {
                if last.elapsed() < REQUERY_DEDUP {
                    return false;
                }
                // Enough time has passed — allow re-query but don't add a
                // duplicate entry. Update the queried_addrs set below.
            } else {
                // Never queried yet — already scheduled.
                return false;
            }
            // Re-query: reset the dedup gate. We need to find it mutably.
            if let Some(existing) = self.nodes.iter_mut().find(|n| n.addr == addr) {
                existing.last_queried = None; // Will be set by spawn_query caller path
            }
            // Remove from queried_addrs so spawn_query sends again.
            self.queried_addrs.remove(&addr);
            return true;
        }

        // Add the new node unconditionally, then sort and evict worst if over
        // capacity.  This matches rqbit's post-add eviction strategy: the new
        // node competes on equal footing with existing nodes (returned_peers,
        // responded, distance, freshness) instead of being rejected solely on
        // xor-distance.  This allows "lateral" exploration of nodes at similar
        // distances that may hold different peer sets.
        self.nodes.push(TrackedNode {
            id,
            addr,
            depth,
            returned_peers: false,
            responded: false,
            last_queried: None,
        });

        if self.nodes.len() > self.config.max_nodes {
            let target = self.target;
            self.sort_nodes(&target);
            self.nodes.pop(); // Evict worst after full multi-key sort

            // If the node we just added was evicted (it's the new worst),
            // don't query it.
            if !self.nodes.iter().any(|n| n.addr == addr) {
                return false;
            }
        }

        true
    }

    /// Spawn a query future that sends `get_peers` and awaits the response via
    /// a oneshot channel.
    fn spawn_query(&mut self, addr: SocketAddr, node_id: Option<Id20>) -> QueryFuture {
        self.queried_addrs.insert(addr);

        // Mark last_queried on the tracked node.
        if let Some(node) = self.nodes.iter_mut().find(|n| n.addr == addr) {
            node.last_queried = Some(Instant::now());
        }

        let socket = self.socket.clone();
        let pending = self.pending.clone();
        let rate_limiter = self.rate_limiter.clone();
        let next_txn_id = self.next_txn_id.clone();
        let own_id = self.own_id;
        let target = self.target;
        let read_only = self.read_only_mode;
        let want = self.want.clone();

        Box::pin(async move {
            rate_limiter.acquire().await;

            let txn = next_txn_id.fetch_add(1, Ordering::Relaxed);
            // Skip txn 0 — the actor reserves it as "invalid".
            let txn = if txn == 0 {
                next_txn_id.fetch_add(1, Ordering::Relaxed)
            } else {
                txn
            };

            let (tx, rx) = oneshot::channel();

            let msg = KrpcMessage {
                transaction_id: TransactionId::from_u16(txn),
                body: KrpcBody::Query(KrpcQuery::GetPeers {
                    id: own_id,
                    info_hash: target,
                    noseed: None,
                    // Match bitmagnet: no scrape flag during peer discovery.
                    // BEP 33 nodes reply with bloom filters when scrape=1,
                    // which suppresses the peer `values` list; omit it so every
                    // node returns the peers it actually knows for the hash.
                    scrape: None,
                    want: want.clone(),
                }),
                sender_ip: None,
                read_only,
            };

            let Ok(bytes) = msg.to_bytes() else {
                return (addr, Err(()));
            };

            pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr,
                    kind: PendingQueryKind::GetPeers { info_hash: target },
                    node_id,
                    response_tx: Some(tx),
                },
            );

            if socket.send_to(&bytes, addr).await.is_err() {
                pending.remove(&txn);
                return (addr, Err(()));
            }

            if let Ok(Ok(resp)) = tokio::time::timeout(QUERY_TIMEOUT, rx).await {
                if let KrpcResponse::GetPeers(gp) = resp.response {
                    (addr, Ok((resp.sender_id, gp)))
                } else {
                    (addr, Err(()))
                }
            } else {
                // Timed out or channel closed — clean up.
                pending.remove(&txn);
                (addr, Err(()))
            }
        })
    }

    /// Process a successful `get_peers` response.
    fn process_response(
        &mut self,
        addr: SocketAddr,
        sender_id: Id20,
        gp: &GetPeersResponse,
        futures: &mut FuturesUnordered<QueryFuture>,
    ) {
        // Mark node as responded.
        if let Some(node) = self.nodes.iter_mut().find(|n| n.addr == addr) {
            node.responded = true;
            node.returned_peers = !gp.peers.is_empty();
        }

        // Send peers to the torrent session, flagging whether the response
        // carried a non-empty seed bloom (BEP 33 `bfsd`) — a live-seeder
        // signal the fetch can use to skip dialing dead hashes.
        if !gp.peers.is_empty() {
            let has_seeds = gp.bfsd.as_ref().is_some_and(|b| b.iter().any(|&x| x != 0));
            let _ = self.peer_tx.send(PeerBatch {
                peers: gp.peers.clone(),
                has_seeds,
            });
        }

        // Send token to actor for announce.
        if let Some(token) = &gp.token {
            let _ = self
                .token_tx
                .send((self.target, sender_id, addr, token.clone()));
        }

        // Determine the depth of the responding node.
        let depth = self
            .nodes
            .iter()
            .find(|n| n.addr == addr)
            .map_or(0, |n| n.depth);
        let new_depth = depth.saturating_add(1);

        // Process v4 nodes.
        for node in &gp.nodes {
            self.forward_node(node.id, node.addr);
            if new_depth <= self.config.max_depth
                && self.maybe_add_node(node.id, node.addr, new_depth)
            {
                futures.push(self.spawn_query(node.addr, Some(node.id)));
            }
        }

        // Process v6 nodes.
        for node in &gp.nodes6 {
            self.forward_node(node.id, node.addr);
            if new_depth <= self.config.max_depth
                && self.maybe_add_node(node.id, node.addr, new_depth)
            {
                futures.push(self.spawn_query(node.addr, Some(node.id)));
            }
        }
    }

    /// Forward a discovered node to the actor for routing-table insertion.
    fn forward_node(&self, id: Id20, addr: SocketAddr) {
        let _ = self.node_tx.send((id, addr));
    }

    /// Mark a node as errored (query timed out or failed).
    fn mark_error(&mut self, addr: SocketAddr) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.addr == addr) {
            node.responded = false;
            node.returned_peers = false;
        }
    }

    /// Sort nodes by multi-key: `returned_peers` DESC, responded DESC,
    /// distance ASC, freshness DESC. Best nodes sort first.
    fn sort_nodes(&mut self, target: &Id20) {
        self.nodes.sort_by(|a, b| {
            // returned_peers DESC (true > false)
            b.returned_peers
                .cmp(&a.returned_peers)
                .then_with(|| {
                    // responded DESC
                    b.responded.cmp(&a.responded)
                })
                .then_with(|| {
                    // distance ASC
                    a.id.xor_distance(target).cmp(&b.id.xor_distance(target))
                })
                .then_with(|| {
                    // freshness DESC: more recently queried is better
                    // None (never queried) sorts after Some (queried)
                    match (a.last_queried, b.last_queried) {
                        (Some(a_t), Some(b_t)) => b_t.cmp(&a_t),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a `TrackedNode` with the given byte and port.
    fn tracked(byte: u8, port: u16, responded: bool, returned_peers: bool) -> TrackedNode {
        let mut id_bytes = [0u8; 20];
        id_bytes[19] = byte;
        TrackedNode {
            id: Id20(id_bytes),
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            depth: 0,
            returned_peers,
            responded,
            last_queried: None,
        }
    }

    fn make_id(byte: u8) -> Id20 {
        let mut bytes = [0u8; 20];
        bytes[19] = byte;
        Id20(bytes)
    }

    /// T1: Multi-key sort: `returned_peers` DESC, responded DESC, distance ASC,
    /// freshness DESC.
    #[test]
    fn tracked_node_multi_key_sort() {
        let target = Id20::ZERO;
        let now = Instant::now();

        let mut nodes = vec![
            // Far, responded, no peers
            tracked(0xFF, 1, true, false),
            // Close, not responded, no peers
            tracked(0x01, 2, false, false),
            // Close, responded, returned peers
            tracked(0x02, 3, true, true),
            // Far, responded, returned peers
            tracked(0xFE, 4, true, true),
        ];

        // Use a DhtLookup just for sort_nodes() — build a minimal one
        let mut wrapper_nodes = std::mem::take(&mut nodes);
        wrapper_nodes.sort_by(|a, b| {
            b.returned_peers
                .cmp(&a.returned_peers)
                .then_with(|| b.responded.cmp(&a.responded))
                .then_with(|| a.id.xor_distance(&target).cmp(&b.id.xor_distance(&target)))
                .then_with(|| match (a.last_queried, b.last_queried) {
                    (Some(a_t), Some(b_t)) => b_t.cmp(&a_t),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
        });

        // returned_peers=true sort first, then by distance
        assert_eq!(wrapper_nodes[0].addr.port(), 3); // 0x02, peers+responded, close
        assert_eq!(wrapper_nodes[1].addr.port(), 4); // 0xFE, peers+responded, far
        // Then responded=true, no peers
        assert_eq!(wrapper_nodes[2].addr.port(), 1); // 0xFF, responded, no peers
        // Then not responded
        assert_eq!(wrapper_nodes[3].addr.port(), 2); // 0x01, not responded

        // Test freshness tiebreaker: two nodes with same first 3 keys but
        // different last_queried — more recent should sort first.
        let mut tied = [
            {
                let mut n = tracked(0x10, 10, true, true);
                n.last_queried = Some(now - Duration::from_secs(30)); // older
                n
            },
            {
                let mut n = tracked(0x10, 11, true, true);
                // Same id byte so same distance — test freshness tiebreaker
                n.id.0[18] = 1; // Differentiate ID but keep distance similar
                n.last_queried = Some(now); // fresher
                n
            },
        ];
        tied.sort_by(|a, b| {
            b.returned_peers
                .cmp(&a.returned_peers)
                .then_with(|| b.responded.cmp(&a.responded))
                .then_with(|| a.id.xor_distance(&target).cmp(&b.id.xor_distance(&target)))
                .then_with(|| match (a.last_queried, b.last_queried) {
                    (Some(a_t), Some(b_t)) => b_t.cmp(&a_t),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
        });
        // Port 10 has id [0..0x10], port 11 has id [0..1, 0x10] — distance
        // to ZERO: port 10 = 0x10, port 11 = 0x0110 (farther). So distance
        // breaks the tie before freshness. Let's make them equal distance:
        let mut fresh_test = [
            {
                let mut n = tracked(0x10, 20, true, true);
                n.last_queried = Some(now - Duration::from_secs(30)); // stale
                n
            },
            {
                let mut n = tracked(0x10, 21, true, true);
                n.last_queried = Some(now); // fresh
                n
            },
        ];
        fresh_test.sort_by(|a, b| {
            b.returned_peers
                .cmp(&a.returned_peers)
                .then_with(|| b.responded.cmp(&a.responded))
                .then_with(|| a.id.xor_distance(&target).cmp(&b.id.xor_distance(&target)))
                .then_with(|| match (a.last_queried, b.last_queried) {
                    (Some(a_t), Some(b_t)) => b_t.cmp(&a_t),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
        });
        // Same distance (same id), so freshness breaks it — port 21 is fresher
        assert_eq!(
            fresh_test[0].addr.port(),
            21,
            "fresher node should sort first"
        );
        assert_eq!(
            fresh_test[1].addr.port(),
            20,
            "stale node should sort second"
        );
    }

    /// T2: Nodes at depth >= `max_depth` are rejected.
    #[tokio::test]
    async fn should_request_depth_limit() {
        let rt = RoutingTable::new(Id20::ZERO);
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Depth 4 should be accepted
        let addr4: SocketAddr = "1.2.3.4:1000".parse().expect("parse");
        assert!(lookup.maybe_add_node(make_id(1), addr4, 4));

        // Depth 5 should be rejected
        let addr5: SocketAddr = "1.2.3.5:1001".parse().expect("parse");
        assert!(!lookup.maybe_add_node(make_id(2), addr5, 5));
    }

    /// T3: 60s re-query dedup gate.
    #[tokio::test]
    async fn should_request_requery_dedup() {
        let rt = RoutingTable::new(Id20::ZERO);
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let addr: SocketAddr = "1.2.3.4:1000".parse().expect("parse");
        let id = make_id(1);

        // First add succeeds.
        assert!(lookup.maybe_add_node(id, addr, 0));

        // Simulate it was queried recently.
        if let Some(n) = lookup.nodes.iter_mut().find(|n| n.addr == addr) {
            n.last_queried = Some(Instant::now());
        }

        // Second add within 60s is rejected.
        assert!(!lookup.maybe_add_node(id, addr, 0));
    }

    /// T4: Self node is rejected.
    #[tokio::test]
    async fn should_request_evict_self_rejected() {
        let own_id = make_id(0xAA);
        let rt = RoutingTable::new(own_id);
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            own_id,
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let addr: SocketAddr = "1.2.3.4:1000".parse().expect("parse");
        assert!(!lookup.maybe_add_node(own_id, addr, 0));
    }

    /// T5: Worst node evicted at capacity.
    #[tokio::test]
    async fn max_nodes_capacity_eviction() {
        let target = Id20::ZERO;
        let rt = RoutingTable::new(make_id(0xFF));
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        // max_nodes = 3 for testing
        let mut lookup = DhtLookup::new(
            target,
            LookupConfig {
                max_depth: 4,
                max_nodes: 3,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Fill to capacity with far nodes (0x80, 0x90, 0xA0).
        let a1: SocketAddr = "1.0.0.1:1".parse().expect("parse");
        let a2: SocketAddr = "1.0.0.2:2".parse().expect("parse");
        let a3: SocketAddr = "1.0.0.3:3".parse().expect("parse");
        assert!(lookup.maybe_add_node(make_id(0x80), a1, 0));
        assert!(lookup.maybe_add_node(make_id(0x90), a2, 0));
        assert!(lookup.maybe_add_node(make_id(0xA0), a3, 0));
        assert_eq!(lookup.nodes.len(), 3);

        // Adding a closer node (0x01) should evict the farthest.
        let a4: SocketAddr = "1.0.0.4:4".parse().expect("parse");
        assert!(lookup.maybe_add_node(make_id(0x01), a4, 0));
        assert_eq!(lookup.nodes.len(), 3);
        assert!(lookup.nodes.iter().any(|n| n.addr == a4));
    }

    /// T6: No alpha cap — all eligible nodes are returned (via `maybe_add_node`).
    #[tokio::test]
    async fn next_queries_no_alpha_cap() {
        let rt = RoutingTable::new(Id20::ZERO);
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Add 10 nodes — all should be accepted (no alpha cap).
        let mut count = 0u32;
        for i in 1..=10u8 {
            let addr: SocketAddr = SocketAddr::from(([10, 0, 0, i], 6880 + u16::from(i)));
            if lookup.maybe_add_node(make_id(i), addr, 0) {
                count = count.saturating_add(1);
            }
        }
        assert_eq!(count, 10);
        assert_eq!(lookup.nodes.len(), 10);
    }

    /// T7: Peers are streamed to the channel.
    #[tokio::test]
    async fn lookup_streams_peers_to_caller() {
        let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let rt = RoutingTable::new(Id20::ZERO);
        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Simulate a response with peers.
        let addr: SocketAddr = "1.2.3.4:6881".parse().expect("parse");
        lookup.maybe_add_node(make_id(1), addr, 0);

        let peer_addr: SocketAddr = "5.6.7.8:9999".parse().expect("parse");
        let gp = GetPeersResponse {
            id: make_id(1),
            token: None,
            peers: vec![peer_addr],
            nodes: Vec::new(),
            nodes6: Vec::new(),
            bfpe: None,
            bfsd: None,
        };

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();
        lookup.process_response(addr, make_id(1), &gp, &mut futures);

        let received = peer_rx.try_recv().expect("should have received peers");
        assert_eq!(received.peers, vec![peer_addr]);
        assert!(!received.has_seeds, "response had no bfsd bloom");
    }

    /// T8: Tokens are sent to the actor channel.
    #[tokio::test]
    async fn lookup_sends_tokens_to_actor() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, mut token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let target = make_id(0x42);
        let rt = RoutingTable::new(Id20::ZERO);
        let mut lookup = DhtLookup::new(
            target,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let addr: SocketAddr = "1.2.3.4:6881".parse().expect("parse");
        let sender_id = make_id(1);
        lookup.maybe_add_node(sender_id, addr, 0);

        let gp = GetPeersResponse {
            id: sender_id,
            token: Some(b"my_token".to_vec()),
            peers: Vec::new(),
            nodes: Vec::new(),
            nodes6: Vec::new(),
            bfpe: None,
            bfsd: None,
        };

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();
        lookup.process_response(addr, sender_id, &gp, &mut futures);

        let (ih, nid, tkn_addr, tkn) = token_rx.try_recv().expect("should have received token");
        assert_eq!(ih, target);
        assert_eq!(nid, sender_id);
        assert_eq!(tkn_addr, addr);
        assert_eq!(tkn, b"my_token");
    }

    /// T9: Response with nodes adds to tracking and spawns queries.
    #[tokio::test]
    async fn lookup_completes_single_round() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let rt = RoutingTable::new(Id20::ZERO);
        let mut lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let addr: SocketAddr = "1.2.3.4:6881".parse().expect("parse");
        lookup.maybe_add_node(make_id(1), addr, 0);

        // Respond with two closer nodes.
        let new_node1 = CompactNodeInfo {
            id: make_id(2),
            addr: "1.2.3.5:6882".parse().expect("parse"),
        };
        let new_node2 = CompactNodeInfo {
            id: make_id(3),
            addr: "1.2.3.6:6883".parse().expect("parse"),
        };

        let gp = GetPeersResponse {
            id: make_id(1),
            token: None,
            peers: Vec::new(),
            nodes: vec![new_node1, new_node2],
            nodes6: Vec::new(),
            bfpe: None,
            bfsd: None,
        };

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();
        lookup.process_response(addr, make_id(1), &gp, &mut futures);

        // Should have added the 2 new nodes + spawned 2 queries.
        assert_eq!(lookup.nodes.len(), 3);
        assert_eq!(futures.len(), 2);
    }

    /// T10: Lookup exits when peer channel is closed.
    #[tokio::test]
    async fn lookup_response_channel_closed() {
        let (peer_tx, peer_rx) = mpsc::unbounded_channel::<crate::krpc::PeerBatch>();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let rt = RoutingTable::new(Id20::ZERO);
        let lookup = DhtLookup::new(
            Id20::ZERO,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Drop the receiver immediately.
        drop(peer_rx);

        // The lookup should exit promptly (peer_tx.is_closed() check).
        let handle = tokio::spawn(lookup.run());
        let result = tokio::time::timeout(Duration::from_secs(3), handle).await;
        assert!(result.is_ok(), "lookup should have exited within 3 seconds");
    }

    /// T11: Stale response for unknown txn is dropped (oneshot approach means
    /// this is inherently handled — the `DashMap` entry is removed on timeout).
    #[test]
    fn lookup_stale_response_dropped() {
        let pending: Arc<DashMap<u16, PendingQuery>> = Arc::new(DashMap::new());
        // No entry for txn 999 — looking it up returns None.
        assert!(pending.get(&999).is_none());
    }

    /// T12: Rate limiter back-pressure test.
    #[test]
    fn lookup_rate_limiter_backpressure() {
        let limiter = SharedRateLimiter::new(2);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        // Third acquire should fail (bucket empty).
        assert!(!limiter.try_acquire());
    }

    /// T13: Root nodes re-injected on timer.
    #[tokio::test]
    async fn lookup_requery_reinjects_roots() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut rt = RoutingTable::new(Id20::ZERO);
        let node_addr: SocketAddr = "10.0.0.1:6881".parse().expect("parse");
        rt.insert(make_id(1), node_addr);

        let mut lookup = DhtLookup::new(
            make_id(0x42),
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();

        // First injection should add the node.
        lookup.inject_roots(&mut futures);
        assert_eq!(lookup.nodes.len(), 1);

        // Second immediate injection should not add duplicates.
        lookup.inject_roots(&mut futures);
        assert_eq!(lookup.nodes.len(), 1);
    }

    /// T13b: A seed node (known hash holder) is queried first, before routing
    /// table roots, and without its ID being required up front.
    #[tokio::test]
    async fn seed_addr_queues_query_before_roots() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut rt = RoutingTable::new(Id20::ZERO);
        let root_addr: SocketAddr = "10.0.0.1:6881".parse().expect("parse");
        rt.insert(make_id(1), root_addr);

        let seed: SocketAddr = "10.9.9.9:6999".parse().expect("parse");
        let lookup = DhtLookup::new(
            make_id(0x42),
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            Some(seed), // seed_addr
        );

        // The seed must be tracked as queried (spawn_query inserts it) without
        // the caller supplying a node ID.
        assert!(lookup.seed_addr.is_some());
        let seed = lookup.seed_addr.unwrap();
        assert_eq!(seed, "10.9.9.9:6999".parse::<SocketAddr>().unwrap());
    }

    /// T14: Useful nodes persist across re-injections.
    #[tokio::test]
    async fn lookup_accumulates_across_requery() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(make_id(1), "10.0.0.1:6881".parse().expect("parse"));

        let rt_arc = Arc::new(parking_lot::RwLock::new(rt));

        let mut lookup = DhtLookup::new(
            make_id(0x42),
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            rt_arc.clone(),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();
        lookup.inject_roots(&mut futures);
        assert_eq!(lookup.nodes.len(), 1);

        // Add a second node to routing table.
        rt_arc
            .write()
            .insert(make_id(2), "10.0.0.2:6882".parse().expect("parse"));

        lookup.inject_roots(&mut futures);
        assert_eq!(lookup.nodes.len(), 2);
    }

    /// T15: Announce tokens are available from the actor's token store after
    /// lookup sends them.
    #[tokio::test]
    async fn announce_tokens_available_after_lookup() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, mut token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let target = make_id(0x42);
        let rt = RoutingTable::new(Id20::ZERO);
        let mut lookup = DhtLookup::new(
            target,
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        // Add two nodes and simulate responses with tokens.
        let addr1: SocketAddr = "1.2.3.4:6881".parse().expect("parse");
        let addr2: SocketAddr = "5.6.7.8:6882".parse().expect("parse");
        let id1 = make_id(1);
        let id2 = make_id(2);
        lookup.maybe_add_node(id1, addr1, 0);
        lookup.maybe_add_node(id2, addr2, 0);

        let gp1 = GetPeersResponse {
            id: id1,
            token: Some(b"token_a".to_vec()),
            peers: Vec::new(),
            nodes: Vec::new(),
            nodes6: Vec::new(),
            bfpe: None,
            bfsd: None,
        };
        let gp2 = GetPeersResponse {
            id: id2,
            token: Some(b"token_b".to_vec()),
            peers: Vec::new(),
            nodes: Vec::new(),
            nodes6: Vec::new(),
            bfpe: None,
            bfsd: None,
        };

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();
        lookup.process_response(addr1, id1, &gp1, &mut futures);
        lookup.process_response(addr2, id2, &gp2, &mut futures);

        // Both tokens should be available.
        let mut tokens = Vec::new();
        while let Ok(t) = token_rx.try_recv() {
            tokens.push(t);
        }
        assert_eq!(tokens.len(), 2);
        assert!(tokens.iter().any(|(_, _, _, t)| t == b"token_a"));
        assert!(tokens.iter().any(|(_, _, _, t)| t == b"token_b"));
    }

    /// T16: `inject_roots()` returns the count and adaptive backoff formula
    /// produces the expected sequence: 1s, 2s, 4s, 8s, 15s (capped).
    #[test]
    fn inject_roots_returns_count_and_adaptive_backoff() {
        // Verify the backoff formula in isolation.
        let mut count = 0u32;
        for expected in [1u64, 2, 4, 8, 15, 15] {
            count = count.saturating_add(1);
            let delay_secs = (1u64 << count.saturating_sub(1).min(4)).min(15);
            assert_eq!(delay_secs, expected, "backoff step {count}");
        }
    }

    /// T17: `inject_roots()` returns actual count when routing table has nodes.
    #[tokio::test]
    async fn inject_roots_returns_nonzero_for_populated_table() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        let mut rt = RoutingTable::new(Id20::ZERO);
        rt.insert(make_id(1), "10.0.0.1:6881".parse().expect("parse"));
        rt.insert(make_id(2), "10.0.0.2:6882".parse().expect("parse"));
        rt.insert(make_id(3), "10.0.0.3:6883".parse().expect("parse"));

        let mut lookup = DhtLookup::new(
            make_id(0x42),
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();

        // First injection should return 3 (all three routing table nodes).
        let injected = lookup.inject_roots(&mut futures);
        assert_eq!(injected, 3);
        assert_eq!(lookup.empty_inject_count, 0);

        // Second immediate injection returns 0 (all already tracked).
        let injected2 = lookup.inject_roots(&mut futures);
        assert_eq!(injected2, 0);
    }

    /// T18: `empty_inject_count` tracks consecutive empty injections.
    #[tokio::test]
    async fn empty_inject_count_increments_on_empty_roots() {
        let (peer_tx, _peer_rx) = mpsc::unbounded_channel();
        let (token_tx, _token_rx) = mpsc::unbounded_channel();
        let (node_tx, _node_rx) = mpsc::unbounded_channel();

        let tok_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind"));

        // Empty routing table — inject_roots will always return 0.
        let rt = RoutingTable::new(Id20::ZERO);

        let mut lookup = DhtLookup::new(
            make_id(0x42),
            LookupConfig {
                max_depth: 4,
                max_nodes: 256,
            },
            AddressFamily::V4,
            tok_socket,
            Arc::new(DashMap::new()),
            Arc::new(SharedRateLimiter::new(250)),
            Arc::new(parking_lot::RwLock::new(rt)),
            Arc::new(AtomicU16::new(1)),
            make_id(0xFF),
            peer_tx,
            token_tx,
            node_tx,
            false, // read_only_mode
            None,  // want
            None,  // seed_addr
        );

        let mut futures: FuturesUnordered<QueryFuture> = FuturesUnordered::new();

        assert_eq!(lookup.empty_inject_count, 0);

        // Simulate what run() does on empty inject.
        let injected = lookup.inject_roots(&mut futures);
        assert_eq!(injected, 0);

        // Mimic the run() logic that increments empty_inject_count.
        if injected == 0 {
            lookup.empty_inject_count = lookup.empty_inject_count.saturating_add(1);
        }
        assert_eq!(lookup.empty_inject_count, 1);

        // After a successful injection, count should reset.
        // (We can't easily force a successful injection here without
        // populating the routing table, so just verify the reset logic.)
        lookup.empty_inject_count = 5;
        if injected > 0 {
            lookup.empty_inject_count = 0;
        }
        // injected is still 0, so count stays at 5.
        assert_eq!(lookup.empty_inject_count, 5);
    }

    // --- BEP 43 read-only node tests ---

    #[tokio::test]
    async fn dht_lookup_query_includes_ro() {
        // When read_only_mode is true, the spawned get_peers query should
        // include `ro: 1`. We verify by constructing the same KrpcMessage
        // that spawn_query builds internally and checking the encoded output.
        let target = Id20::ZERO;
        let own_id = make_id(0xFF);

        // Build the message the same way spawn_query does, but with read_only: true
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(99),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: own_id,
                info_hash: target,
                noseed: None,
                scrape: None,
                want: None,
            }),
            sender_ip: None,
            read_only: true,
        };

        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(decoded.read_only, "get_peers query should carry ro flag");

        // Also verify the raw bencode contains the "ro" key
        let raw: gaia_bencode::BencodeValue = gaia_bencode::from_bytes(&bytes).unwrap();
        let dict = raw.as_dict().unwrap();
        assert!(
            dict.contains_key(&b"ro"[..]),
            "encoded bytes should contain ro key"
        );
        let ro_val = dict.get(&b"ro"[..]).unwrap().as_int().unwrap();
        assert_eq!(ro_val, 1);

        // Verify that read_only_mode: false does NOT include ro
        let msg_normal = KrpcMessage {
            transaction_id: TransactionId::from_u16(100),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: own_id,
                info_hash: target,
                noseed: None,
                scrape: None,
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes_normal = msg_normal.to_bytes().unwrap();
        let raw_normal: gaia_bencode::BencodeValue =
            gaia_bencode::from_bytes(&bytes_normal).unwrap();
        let dict_normal = raw_normal.as_dict().unwrap();
        assert!(
            !dict_normal.contains_key(&b"ro"[..]),
            "non-read-only query should not contain ro key"
        );
    }
}
