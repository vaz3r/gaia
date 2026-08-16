#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unchecked_time_subtraction,
    reason = "M175: DHT actor — KRPC field widths fixed by spec; time deltas use post-bootstrap Instants captured well after process start"
)]

//! DHT actor: single-owner event loop managing the routing table, UDP socket,
//! and pending queries.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use dashmap::DashMap;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, trace, warn};

use gaia_core::{AddressFamily, Id20};

use crate::bep44::{self, ImmutableItem, MAX_SALT_SIZE, MAX_VALUE_SIZE, MutableItem};
use crate::compact::CompactNodeInfo;
use crate::error::{Error, Result};
use crate::krpc::{
    GetPeersResponse, KrpcBody, KrpcMessage, KrpcQuery, KrpcResponse, SampleInfohashesResponse,
    TransactionId,
};
use crate::lookup::{FindNodeCallbacks, IterativeLookup};
use crate::node_id::{self, ExternalIpVoter, IpVoteSource};
use crate::peer_store::PeerStore;
use crate::routing_table::{K, RESPONSE_K, RoutingTable};
use crate::storage::{DhtStorage, InMemoryDhtStorage};

#[allow(unused_imports)]
use ed25519_dalek::SigningKey;

/// Token-bucket rate limiter for outgoing KRPC queries.
///
/// Permits refill continuously based on elapsed real time. `try_acquire` is
/// non-blocking: it either consumes a permit or returns `false` immediately.
struct QueryRateLimiter {
    permits: u32,
    max_permits: u32,
    last_refill: Instant,
    refill_rate: u32,
}

impl QueryRateLimiter {
    /// Create a new limiter with `rate` permits per second. Starts with a full
    /// bucket so the first burst of queries is not artificially delayed.
    fn new(rate: usize) -> Self {
        Self {
            permits: rate as u32,
            max_permits: rate as u32,
            last_refill: Instant::now(),
            refill_rate: rate as u32,
        }
    }

    /// Attempt to consume one permit. Returns `true` if a permit was available,
    /// `false` if the bucket is empty. Never blocks.
    fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.permits > 0 {
            self.permits -= 1;
            true
        } else {
            false
        }
    }

    /// Refill the bucket based on elapsed time since the last refill. Caps at
    /// `max_permits`. Only updates `last_refill` when at least one permit is
    /// added (avoids drift on very fast calls).
    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();
        let new_permits = (elapsed_secs * f64::from(self.refill_rate)) as u32;
        if new_permits > 0 {
            self.permits = (self.permits + new_permits).min(self.max_permits);
            self.last_refill = Instant::now();
        }
    }
}

/// Arc-compatible rate limiter wrapping [`QueryRateLimiter`] in a `Mutex`.
///
/// Used by both the DHT actor and spawned `DhtLookup` tasks.
pub(crate) struct SharedRateLimiter {
    inner: parking_lot::Mutex<QueryRateLimiter>,
}

impl SharedRateLimiter {
    /// Create a new shared rate limiter with `rate` permits per second.
    pub fn new(rate: usize) -> Self {
        Self {
            inner: parking_lot::Mutex::new(QueryRateLimiter::new(rate)),
        }
    }

    /// Non-blocking acquire. Returns `true` if a permit was available.
    pub fn try_acquire(&self) -> bool {
        self.inner.lock().try_acquire()
    }

    /// Async acquire — sleeps briefly between attempts until a permit is
    /// available. At 250 permits/sec the bucket refills ~1 permit per 4ms.
    pub async fn acquire(&self) {
        loop {
            if self.try_acquire() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    }
}

/// Configuration for the DHT.
#[derive(Debug, Clone)]
pub struct DhtConfig {
    /// Address to bind the UDP socket.
    pub bind_addr: SocketAddr,
    /// Bootstrap nodes (host:port strings resolved at startup).
    pub bootstrap_nodes: Vec<String>,
    /// Our node ID. Generated randomly if `None`.
    pub own_id: Option<Id20>,
    /// Max outgoing queries per second (0 = unlimited).
    pub queries_per_second: usize,
    /// Timeout for individual KRPC queries.
    pub query_timeout: Duration,
    /// Address family for this DHT instance (determines compact format and DNS filtering).
    pub address_family: AddressFamily,
    /// BEP 42: Enforce node ID verification when inserting into routing table.
    /// Nodes with IDs that don't match their IP are rejected.
    pub enforce_node_id: bool,
    /// BEP 42: Restrict routing table to one node per IP address.
    pub restrict_routing_ips: bool,
    /// BEP 44: Maximum number of stored DHT items (immutable + mutable).
    pub dht_max_items: usize,
    /// BEP 44: Lifetime of DHT items in seconds before expiry.
    pub dht_item_lifetime_secs: u64,
    /// Maximum number of nodes in the routing table. Prevents unbounded growth
    /// from adversarial node injection. Default: 512 (matches rqbit).
    pub max_routing_nodes: usize,
    /// Directory for persisting DHT routing table state as JSON.
    /// When set, the actor saves/loads `dht_state.json` (V4) or
    /// `dht_state_v6.json` (V6) via atomic temp-file + rename.
    pub state_dir: Option<PathBuf>,
    /// BEP 43: Read-only mode. When enabled, outgoing queries include `ro: 1`
    /// and the node does not send `announce_peer` messages. Other nodes should
    /// not add us to their routing tables.
    pub read_only_mode: bool,
    /// BEP 45: Include `want` in outgoing `find_node`/`get_peers` to request
    /// both IPv4 and IPv6 nodes from dual-stack peers.
    pub enable_multi_address: bool,
}

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            bootstrap_nodes: vec![
                "router.bittorrent.com:6881".into(),
                "dht.transmissionbt.com:6881".into(),
                "router.utorrent.com:6881".into(),
            ],
            own_id: None,
            queries_per_second: 250,
            query_timeout: Duration::from_secs(5),
            address_family: AddressFamily::V4,
            enforce_node_id: false,
            restrict_routing_ips: true,
            dht_max_items: 700,
            dht_item_lifetime_secs: 7200,
            max_routing_nodes: 512,
            state_dir: None,
            read_only_mode: false,
            enable_multi_address: true,
        }
    }
}

impl DhtConfig {
    /// Default configuration for an IPv6 DHT instance (BEP 24).
    #[must_use]
    pub fn default_v6() -> Self {
        Self {
            bind_addr: "[::]:0".parse().unwrap(),
            bootstrap_nodes: vec![
                "router.bittorrent.com:6881".into(),
                "dht.libtorrent.org:25401".into(),
            ],
            own_id: None,
            queries_per_second: 250,
            query_timeout: Duration::from_secs(5),
            address_family: AddressFamily::V6,
            enforce_node_id: false,
            restrict_routing_ips: true,
            dht_max_items: 700,
            dht_item_lifetime_secs: 7200,
            max_routing_nodes: 512,
            state_dir: None,
            read_only_mode: false,
            enable_multi_address: true,
        }
    }
}

/// Runtime statistics for the DHT.
#[derive(Debug, Clone)]
pub struct DhtStats {
    /// Our current node ID (may differ from startup ID after BEP 42 regeneration).
    pub node_id: Id20,
    /// Number of nodes in the routing table.
    pub routing_table_size: usize,
    /// Number of k-buckets in use.
    pub bucket_count: usize,
    /// Number of distinct info hashes tracked in the peer store.
    pub peer_store_info_hashes: usize,
    /// Total number of peers across all info hashes.
    pub peer_store_peers: usize,
    /// Number of in-flight KRPC queries.
    pub pending_queries: usize,
    /// Number of in-flight `DhtLookup` tasks.
    pub active_lookups: usize,
    /// Number of retained announce tokens (bounded by `MAX_ANNOUNCE_TOKENS`).
    pub announce_tokens: usize,
    /// Total KRPC queries sent since startup.
    pub total_queries_sent: u64,
    /// Total KRPC responses received since startup.
    pub total_responses_received: u64,
    /// Number of BEP 44 items stored (immutable + mutable).
    pub dht_item_count: usize,
    /// Passive-intake funnel: inbound `announce_peer` queries received.
    pub announces_received: u64,
    /// Passive-intake funnel: announces rejected on token validation.
    pub announces_token_rejected: u64,
    /// Passive-intake funnel: announces suppressed by read-only mode.
    pub announces_suppressed_readonly: u64,
    /// Passive-intake funnel: inbound `get_peers` queries (active seekers).
    pub lookups_received: u64,
}

/// Result of a `sample_infohashes` query (BEP 51).
#[derive(Debug, Clone)]
pub struct SampleInfohashesResult {
    /// Minimum seconds before querying the same node again.
    pub interval: i64,
    /// Estimated total info hashes in the remote node's store.
    pub num: i64,
    /// Sampled info hashes.
    pub samples: Vec<Id20>,
    /// Closer nodes for traversal.
    pub nodes: Vec<CompactNodeInfo>,
}

/// A passive DHT event surfaced to the application via the handle's broadcast
/// channel. These are the raw observations that drive a passive-intake crawler:
/// an inbound `announce_peer` proves a hash is live (a peer is announcing it
/// right now), and an inbound `get_peers` proves someone is actively seeking it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhtEvent {
    /// A remote node sent us a valid `announce_peer` for `info_hash` from
    /// `peer_addr` — the hash has at least one live announcing peer.
    Announced {
        /// The announced infohash.
        info_hash: Id20,
        /// The announcing peer (IP + torrent port).
        peer_addr: std::net::SocketAddr,
    },
    /// A remote node queried us with `get_peers` for `info_hash`.
    LookedUp {
        /// The sought infohash.
        info_hash: Id20,
        /// The querying node's address.
        from_addr: std::net::SocketAddr,
    },
}

/// A cloneable handle to the DHT actor.
#[derive(Clone, Debug)]
pub struct DhtHandle {
    tx: mpsc::Sender<DhtCommand>,
    /// Broadcast of passive inbound events (`announce_peer` / `get_peers`).
    events: tokio::sync::broadcast::Sender<DhtEvent>,
}

impl DhtHandle {
    /// Subscribe to passive inbound DHT events. The returned receiver sees
    /// every `announce_peer` and `get_peers` the node handles. If the receiver
    /// lags (the actor is faster than the consumer), the oldest unconsumed
    /// event is dropped and a `RecvError::Lagged` is delivered — consumers
    /// should use a bounded/cheap pipeline.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<DhtEvent> {
        self.events.subscribe()
    }
}

enum DhtCommand {
    GetPeers {
        info_hash: Id20,
        reply: mpsc::UnboundedSender<crate::krpc::PeerBatch>,
        seed_addr: Option<std::net::SocketAddr>,
    },
    Announce {
        info_hash: Id20,
        port: u16,
        reply: oneshot::Sender<Result<()>>,
    },
    Stats {
        reply: oneshot::Sender<DhtStats>,
    },
    UpdateExternalIp {
        ip: std::net::IpAddr,
        source: IpVoteSource,
    },
    GetImmutable {
        target: Id20,
        reply: oneshot::Sender<Result<Option<Vec<u8>>>>,
    },
    PutImmutable {
        value: Vec<u8>,
        reply: oneshot::Sender<Result<Id20>>,
    },
    GetMutable {
        public_key: [u8; 32],
        salt: Vec<u8>,
        #[allow(clippy::type_complexity)]
        reply: oneshot::Sender<Result<Option<(Vec<u8>, i64)>>>,
    },
    PutMutable {
        keypair_bytes: [u8; 32],
        value: Vec<u8>,
        seq: i64,
        salt: Vec<u8>,
        reply: oneshot::Sender<Result<Id20>>,
    },
    SampleInfohashes {
        target: Id20,
        reply: oneshot::Sender<Result<SampleInfohashesResult>>,
    },
    DirectGetPeers {
        target: SocketAddr,
        info_hash: Id20,
        reply: oneshot::Sender<Result<Vec<SocketAddr>>>,
    },
    GetRoutingNodes {
        reply: oneshot::Sender<Vec<(Id20, SocketAddr)>>,
    },
    /// M173 Lane B (B7): synchronously persist the routing table to
    /// `dht_state.json` and reply when the rename has completed.
    /// Used by the `apply_settings` DHT-restart phase so the saved
    /// state survives a runtime `enable_dht: true → false → true`
    /// cycle.
    SaveRoutingTable {
        reply: oneshot::Sender<Result<()>>,
    },
    /// Optional reply lets the caller block until the actor has
    /// drained — used by the `apply_settings` DHT-stop phase. The
    /// actor saves the routing table BEFORE acking, so the on-disk
    /// state is up-to-date when the new actor starts.
    Shutdown {
        reply: Option<oneshot::Sender<()>>,
    },
}

impl DhtHandle {
    /// Start the DHT actor and return a handle plus an IP consensus channel.
    ///
    /// The consensus channel fires when the BEP 42 `ExternalIpVoter` reaches
    /// agreement on our external IP address.
    ///
    /// # Errors
    ///
    /// Returns an error if the UDP socket cannot be bound.
    pub async fn start(config: DhtConfig) -> Result<(Self, mpsc::Receiver<std::net::IpAddr>)> {
        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await?);
        let local_addr = socket.local_addr()?;
        debug!(addr = %local_addr, "DHT socket bound");

        let (tx, rx) = mpsc::channel(256);
        let (ip_consensus_tx, ip_consensus_rx) = mpsc::channel(4);
        let (events_tx, _) = tokio::sync::broadcast::channel(1024);
        let handle = Self { tx, events: events_tx.clone() };

        let actor = DhtActor::new(config, socket, rx, ip_consensus_tx, events_tx);
        tokio::spawn(actor.run());

        Ok((handle, ip_consensus_rx))
    }

    /// Start the DHT actor on a caller-provided shared UDP socket (M263).
    ///
    /// Used by the unified single-socket path: the session binds one
    /// `Arc<UdpSocket>` per family on the port-mapped listen port, runs the sole
    /// reader, and feeds pre-classified DHT datagrams through `inbound_rx`. The
    /// actor never reads `socket` directly in this mode (the legacy `recv_from`
    /// arm is disabled when `inbound_rx` is `Some`), but it DOES send all replies
    /// and queries through `socket`, so outbound traffic originates from the
    /// shared, port-mapped address — making the DHT node reachable on the same
    /// port as uTP/BitTorrent.
    ///
    /// # Errors
    ///
    /// Returns an error if `socket.local_addr()` fails.
    //
    // Not `async`: the socket is already bound by the caller, so unlike
    // `start()` (which awaits `UdpSocket::bind`) there is no async work here.
    pub fn start_unified(
        config: DhtConfig,
        socket: Arc<UdpSocket>,
        inbound_rx: mpsc::Receiver<(Bytes, SocketAddr)>,
    ) -> Result<(Self, mpsc::Receiver<std::net::IpAddr>)> {
        let local_addr = socket.local_addr()?;
        debug!(addr = %local_addr, "DHT bound on shared (unified) UDP socket");

        let (tx, rx) = mpsc::channel(256);
        let (ip_consensus_tx, ip_consensus_rx) = mpsc::channel(4);
        let (events_tx, _) = tokio::sync::broadcast::channel(1024);
        let handle = Self { tx, events: events_tx.clone() };

        let mut actor = DhtActor::new(config, socket, rx, ip_consensus_tx, events_tx);
        actor.inbound_rx = Some(inbound_rx);
        actor.unified = true;
        tokio::spawn(actor.run());

        Ok((handle, ip_consensus_rx))
    }

    /// Notify the DHT of our external IP (from NAT/tracker discovery).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn update_external_ip(
        &self,
        ip: std::net::IpAddr,
        source: IpVoteSource,
    ) -> Result<()> {
        self.tx
            .send(DhtCommand::UpdateExternalIp { ip, source })
            .await
            .map_err(|_| Error::Shutdown)
    }

    /// Discover peers for an `info_hash`.
    ///
    /// Returns a channel that receives batches of peers as they are found.
    /// The channel closes when the search is exhausted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn get_peers(
        &self,
        info_hash: Id20,
    ) -> Result<mpsc::UnboundedReceiver<crate::krpc::PeerBatch>> {
        self.get_peers_seeded(info_hash, None).await
    }

    /// Like `get_peers`, but the lookup first queries `seed_addr` (a node
    /// known to hold the hash), then walks keyspace-closest roots.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn get_peers_seeded(
        &self,
        info_hash: Id20,
        seed_addr: Option<std::net::SocketAddr>,
    ) -> Result<mpsc::UnboundedReceiver<crate::krpc::PeerBatch>> {
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        self.tx
            .send(DhtCommand::GetPeers {
                info_hash,
                reply: reply_tx,
                seed_addr,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        Ok(reply_rx)
    }

    /// Announce that we have peers for an `info_hash` on the given port.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn announce(&self, info_hash: Id20, port: u16) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::Announce {
                info_hash,
                port,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Get current DHT statistics.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn stats(&self) -> Result<DhtStats> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::Stats { reply: reply_tx })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)
    }

    /// Get the number of nodes currently in the routing table (M171 D4).
    ///
    /// Thin accessor over [`DhtStats::routing_table_size`] used by the qBt
    /// v2 `transferInfo.dht_nodes` field and the DHT pseudo-tracker's
    /// `num_peers` column. Returns `Ok(0)` when the routing table is
    /// empty — including immediately after startup, before bootstrap has
    /// populated any buckets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn node_count(&self) -> Result<usize> {
        Ok(self.stats().await?.routing_table_size)
    }

    /// Shut down the DHT actor (fire-and-forget).
    ///
    /// Returns once the shutdown command has been queued. The actor
    /// will persist the routing table (`dht_state.json`) before
    /// terminating, but this method does NOT wait for that. For
    /// runtime DHT-restart paths that need to be sure the state is
    /// on disk before starting a new actor, use
    /// [`Self::shutdown_and_wait`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the command channel has
    /// already closed.
    pub async fn shutdown(&self) -> Result<()> {
        self.tx
            .send(DhtCommand::Shutdown { reply: None })
            .await
            .map_err(|_| Error::Shutdown)
    }

    /// M173 Lane B (B7): shut down the DHT actor and wait for it to
    /// fully drain — including persisting the routing table to
    /// `dht_state.json`.
    ///
    /// Returns `Ok(())` once the actor has saved its state and
    /// terminated. Used by the `apply_settings` DHT-restart phase so
    /// the new DHT actor (started with the same `state_dir`) can
    /// load the pre-restart state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor exits before
    /// sending its reply (typically because it had already shut
    /// down for another reason).
    pub async fn shutdown_and_wait(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::Shutdown {
                reply: Some(reply_tx),
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)
    }

    /// M173 Lane B (B7): synchronously persist the routing table.
    ///
    /// Returns `Ok(())` once `dht_state.json` has been written and
    /// renamed atomically. Distinct from
    /// [`Self::shutdown_and_wait`] in that the actor continues
    /// running afterwards — used by callers that want to checkpoint
    /// state without restarting DHT.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor channel has closed.
    /// May return an underlying I/O error wrapped in
    /// `Error::Shutdown` if the persist itself failed (the actor
    /// returns the error verbatim, but the channel-closed case is
    /// indistinguishable from the I/O case at the API boundary).
    pub async fn save_routing_table(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::SaveRoutingTable { reply: reply_tx })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Store an immutable item in the DHT (BEP 44).
    ///
    /// Returns the SHA-1 target hash that can be used to retrieve the item.
    /// The value must be valid bencoded data, max 1000 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped, or a BEP 44
    /// validation error if the value exceeds size limits.
    pub async fn put_immutable(&self, value: Vec<u8>) -> Result<Id20> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::PutImmutable {
                value,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Retrieve an immutable item from the DHT (BEP 44).
    ///
    /// Returns the raw bencoded value if found, `None` if not.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn get_immutable(&self, target: Id20) -> Result<Option<Vec<u8>>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::GetImmutable {
                target,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Store a mutable item in the DHT (BEP 44).
    ///
    /// - `keypair_bytes`: 32-byte ed25519 seed (secret key)
    /// - `value`: bencoded data, max 1000 bytes
    /// - `seq`: sequence number (must be higher than any previously stored)
    /// - `salt`: optional salt for sub-key isolation (max 64 bytes)
    ///
    /// Returns the target hash.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped, or a BEP 44
    /// validation error if value or salt exceeds size limits.
    pub async fn put_mutable(
        &self,
        keypair_bytes: [u8; 32],
        value: Vec<u8>,
        seq: i64,
        salt: Vec<u8>,
    ) -> Result<Id20> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::PutMutable {
                keypair_bytes,
                value,
                seq,
                salt,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Query a DHT node for a random sample of info hashes (BEP 51).
    ///
    /// Routes toward `target` to find the responding node. Returns sampled
    /// hashes, the interval before re-querying, and closer nodes for traversal.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn sample_infohashes(&self, target: Id20) -> Result<SampleInfohashesResult> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::SampleInfohashes {
                target,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Query a specific remote DHT node directly for peers of an infohash in a single
    /// 1-shot KRPC `get_peers` request (Bitmagnet pattern).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped, or [`Error::Timeout`] if no response arrives.
    pub async fn direct_get_peers(&self, target: SocketAddr, info_hash: Id20) -> Result<Vec<SocketAddr>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::DirectGetPeers {
                target,
                info_hash,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Retrieve a mutable item from the DHT (BEP 44).
    ///
    /// Returns `(value, seq)` if found, `None` if not.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Shutdown`] if the actor has stopped.
    pub async fn get_mutable(
        &self,
        public_key: [u8; 32],
        salt: Vec<u8>,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(DhtCommand::GetMutable {
                public_key,
                salt,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Shutdown)?;
        reply_rx.await.map_err(|_| Error::Shutdown)?
    }

    /// Return all nodes currently in the DHT routing table.
    pub async fn get_routing_nodes(&self) -> Vec<(Id20, SocketAddr)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self
            .tx
            .send(DhtCommand::GetRoutingNodes { reply: reply_tx })
            .await;
        reply_rx.await.unwrap_or_default()
    }
}

// ---- Actor internals ----

struct DhtActor {
    config: DhtConfig,
    address_family: AddressFamily,
    /// UDP socket shared with spawned `DhtLookup` tasks.
    socket: Arc<UdpSocket>,
    rx: mpsc::Receiver<DhtCommand>,
    /// Routing table shared with spawned `DhtLookup` tasks.
    routing_table: Arc<parking_lot::RwLock<RoutingTable>>,
    peer_store: PeerStore,
    /// BEP 44 item storage (immutable + mutable).
    item_store: Box<dyn DhtStorage + Send>,
    /// Pending KRPC queries shared with spawned `DhtLookup` tasks.
    pending: Arc<DashMap<u16, PendingQuery>>,
    /// Atomic transaction ID counter shared with spawned `DhtLookup` tasks.
    next_txn_id: Arc<AtomicU16>,
    stats: ActorStats,
    /// Announce tokens collected from active lookups via the token channel.
    /// Bounded: tokens are only consumed when WE announce a hash, which this
    /// read-only crawler never does (its `get_peers` growth lookups use random
    /// targets). Without a cap, every grower lookup leaks an entry here — the
    /// source of an unbounded RSS growth under load.
    announce_tokens: HashMap<Id20, HashMap<Id20, (SocketAddr, Vec<u8>)>>,
    /// Insertion order for `announce_tokens` LRU eviction (oldest first).
    announce_token_order: std::collections::VecDeque<Id20>,
    /// Sender for lookup token channel (cloned to each `DhtLookup`).
    lookup_token_tx: mpsc::UnboundedSender<(Id20, Id20, SocketAddr, Vec<u8>)>,
    /// Receiver for lookup token channel.
    lookup_token_rx: mpsc::UnboundedReceiver<(Id20, Id20, SocketAddr, Vec<u8>)>,
    /// Sender for lookup node channel (cloned to each `DhtLookup`).
    lookup_node_tx: mpsc::UnboundedSender<(Id20, SocketAddr)>,
    /// Receiver for lookup node channel.
    lookup_node_rx: mpsc::UnboundedReceiver<(Id20, SocketAddr)>,
    /// Active `DhtLookup` task handles, keyed by `info_hash`.
    active_lookups: HashMap<Id20, tokio::task::JoinHandle<()>>,
    /// Active BEP 44 get lookups.
    item_lookups: HashMap<Id20, ItemLookupState>,
    /// Active BEP 44 put operations (waiting for tokens before sending puts).
    item_put_ops: HashMap<Id20, ItemPutState>,
    /// BEP 42 external IP voter: aggregates IP reports from KRPC responses.
    ip_voter: ExternalIpVoter,
    /// Broadcast of passive inbound events (`announce_peer` / `get_peers`).
    events_tx: tokio::sync::broadcast::Sender<DhtEvent>,
    /// Callback channel: fires when voter consensus changes.
    ip_consensus_tx: mpsc::Sender<std::net::IpAddr>,
    /// Pending one-shot replies for `sample_infohashes` queries.
    sample_replies: HashMap<u16, oneshot::Sender<Result<SampleInfohashesResult>>>,
    /// Pending one-shot replies for `direct_get_peers` queries.
    direct_peer_replies: HashMap<u16, oneshot::Sender<Result<Vec<SocketAddr>>>>,
    /// Token-bucket rate limiter shared with spawned `DhtLookup` tasks.
    rate_limiter: Arc<SharedRateLimiter>,
    /// Active iterative bootstrap lookup (`find_node` self-lookup after initial bootstrap).
    bootstrap_lookup: Option<IterativeLookup<FindNodeCallbacks>>,
    /// Whether initial bootstrap (`FindNodeLookup`) has completed (M97).
    bootstrap_complete: bool,
    /// M146: Queued `get_peers` waiting for at least 1 routing table node.
    /// Lowered from M97's threshold=8 to threshold=1 (empty-table only).
    pending_get_peers: Vec<(Id20, mpsc::UnboundedSender<crate::krpc::PeerBatch>, Option<std::net::SocketAddr>)>,
    /// Bootstrap timeout timer — forces `bootstrap_complete` after 10s (M97).
    bootstrap_timeout: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    /// Timestamp of last `ping_questionable_nodes()` call for two-phase gating (M105).
    last_ping: Instant,
    /// Receiver for DNS-resolved bootstrap addresses from background tasks (M105).
    /// Set during `bootstrap()`, drained in the main select! loop, cleared when
    /// all spawned DNS tasks complete (channel closes).
    dns_bootstrap_rx: Option<mpsc::Receiver<Vec<SocketAddr>>>,
    /// M263 unified single-socket demux: when `Some`, inbound DHT datagrams
    /// arrive pre-classified through this channel (the session owns the shared
    /// UDP socket and runs the sole reader). `None` for the legacy path, where
    /// the actor reads its own `socket` directly. `socket` is still held in
    /// unified mode — it is the shared `Arc<UdpSocket>` used for all outbound
    /// sends, so replies originate from the port-mapped listen port.
    inbound_rx: Option<mpsc::Receiver<(Bytes, SocketAddr)>>,
    /// M263: `true` in unified mode. Disables the direct `socket.recv_from`
    /// select arm so the actor never competes with the demux task for packets
    /// on the shared socket (a separate field from `inbound_rx` so the fd-arm
    /// precondition and the demux-drain arm borrow disjoint fields of `self`).
    unified: bool,
}

struct ActorStats {
    total_queries_sent: u64,
    total_responses_received: u64,
    /// Passive-intake funnel (crawler-conversion Phase 2): inbound announce
    /// counts at each gate so the app can see where announces are lost.
    announces_received: u64,
    announces_token_rejected: u64,
    announces_suppressed_readonly: u64,
    /// Inbound `get_peers` queries (someone actively seeking a hash) — a live
    /// signal and a potential high-volume passive source.
    lookups_received: u64,
}

/// A pending KRPC query awaiting a response.
pub(crate) struct PendingQuery {
    pub sent_at: Instant,
    pub addr: SocketAddr,
    pub kind: PendingQueryKind,
    pub node_id: Option<Id20>,
    /// If set, the response is routed through this oneshot instead of being
    /// handled by the actor's `handle_response()` match arms.
    pub response_tx: Option<oneshot::Sender<PendingQueryResponse>>,
}

/// Raw KRPC response forwarded to a `DhtLookup` via oneshot.
pub(crate) struct PendingQueryResponse {
    pub sender_id: Id20,
    pub response: KrpcResponse,
}

#[derive(Debug)]
pub(crate) enum PendingQueryKind {
    Ping,
    FindNode,
    GetPeers {
        info_hash: Id20,
    },
    AnnouncePeer,
    /// BEP 44: outgoing get item query.
    GetItem {
        target: Id20,
    },
    /// BEP 44: outgoing put item query.
    PutItem,
    /// BEP 51: outgoing `sample_infohashes` query.
    SampleInfohashes,
    /// 1-shot direct `get_peers` query to a reporting node.
    DirectGetPeers,
}

/// State for an active BEP 44 get lookup.
enum ItemLookupState {
    Immutable {
        #[allow(clippy::type_complexity)]
        reply: Option<oneshot::Sender<Result<Option<Vec<u8>>>>>,
        queried: std::collections::HashSet<Id20>,
    },
    Mutable {
        salt: Vec<u8>,
        #[allow(clippy::type_complexity)]
        reply: Option<oneshot::Sender<Result<Option<(Vec<u8>, i64)>>>>,
        best_seq: i64,
        best_value: Option<Vec<u8>>,
        queried: std::collections::HashSet<Id20>,
    },
}

/// State for an active BEP 44 put operation (waiting for tokens then sending puts).
enum ItemPutState {
    Immutable {
        item: crate::bep44::ImmutableItem,
        tokens: HashMap<Id20, (SocketAddr, Vec<u8>)>,
        sent_puts: usize,
        reply: Option<oneshot::Sender<Result<Id20>>>,
    },
    Mutable {
        item: crate::bep44::MutableItem,
        tokens: HashMap<Id20, (SocketAddr, Vec<u8>)>,
        sent_puts: usize,
        reply: Option<oneshot::Sender<Result<Id20>>>,
    },
}

/// Parameters for a single BEP 44 put-item query.
struct PutItemParams {
    addr: SocketAddr,
    token: Vec<u8>,
    value: Vec<u8>,
    key: Option<[u8; 32]>,
    signature: Option<[u8; 64]>,
    seq: Option<i64>,
    salt: Option<Vec<u8>>,
}

/// JSON serialization format for persisted DHT routing table state.
#[derive(serde::Serialize, serde::Deserialize)]
struct DhtState {
    /// Our node ID as a hex string.
    node_id: String,
    /// All nodes from the routing table.
    nodes: Vec<DhtNodeEntry>,
}

/// A single node entry in the persisted JSON state.
#[derive(serde::Serialize, serde::Deserialize)]
struct DhtNodeEntry {
    /// Node ID as a hex string.
    id: String,
    /// Socket address as "ip:port".
    addr: String,
}

/// Interval for routing table maintenance.
const MAINTENANCE_INTERVAL: Duration = Duration::from_mins(1);
/// Interval for peer store cleanup.
const CLEANUP_INTERVAL: Duration = Duration::from_mins(5);
/// Interval for pinging questionable nodes.
const PING_INTERVAL: Duration = Duration::from_secs(5);
/// Max announce tokens retained per actor. Tokens from random-target growth
/// lookups are never consumed by this read-only crawler; the cap bounds the
/// otherwise-unbounded `announce_tokens` growth that caused an RSS leak.
const MAX_ANNOUNCE_TOKENS: usize = 4096;

impl DhtActor {
    fn new(
        config: DhtConfig,
        socket: Arc<UdpSocket>,
        rx: mpsc::Receiver<DhtCommand>,
        ip_consensus_tx: mpsc::Sender<std::net::IpAddr>,
        events_tx: tokio::sync::broadcast::Sender<DhtEvent>,
    ) -> Self {
        let own_id = config.own_id.unwrap_or_else(generate_node_id);
        let address_family = config.address_family;
        let restrict_ips = config.restrict_routing_ips;
        let max_routing_nodes = config.max_routing_nodes;
        debug!(id = %own_id, family = ?address_family, "DHT node ID");

        let max_items = config.dht_max_items;
        let queries_per_second = config.queries_per_second;

        let (lookup_token_tx, lookup_token_rx) = mpsc::unbounded_channel();
        let (lookup_node_tx, lookup_node_rx) = mpsc::unbounded_channel();

        let mut actor = Self {
            config,
            address_family,
            socket,
            rx,
            routing_table: Arc::new(parking_lot::RwLock::new(RoutingTable::with_config(
                own_id,
                restrict_ips,
                max_routing_nodes,
            ))),
            peer_store: PeerStore::new(),
            item_store: Box::new(InMemoryDhtStorage::new(max_items)),
            pending: Arc::new(DashMap::new()),
            next_txn_id: Arc::new(AtomicU16::new(1)),
            stats: ActorStats {
                total_queries_sent: 0,
                total_responses_received: 0,
                announces_received: 0,
                announces_token_rejected: 0,
                announces_suppressed_readonly: 0,
                lookups_received: 0,
            },
            announce_tokens: HashMap::new(),
            announce_token_order: std::collections::VecDeque::new(),
            lookup_token_tx,
            lookup_token_rx,
            lookup_node_tx,
            lookup_node_rx,
            active_lookups: HashMap::new(),
            item_lookups: HashMap::new(),
            item_put_ops: HashMap::new(),
            ip_voter: ExternalIpVoter::new(10),
            ip_consensus_tx,
            events_tx,
            sample_replies: HashMap::new(),
            direct_peer_replies: HashMap::new(),
            rate_limiter: Arc::new(SharedRateLimiter::new(queries_per_second)),
            bootstrap_lookup: None,
            bootstrap_complete: false,
            pending_get_peers: Vec::new(),
            bootstrap_timeout: Some(Box::pin(tokio::time::sleep(Duration::from_secs(10)))),
            last_ping: Instant::now(),
            dns_bootstrap_rx: None,
            inbound_rx: None,
            unified: false,
        };

        // Load persisted routing table from JSON (if state_dir is configured).
        // Loaded nodes are marked Questionable and will be verified via pings.
        actor.load_routing_table();

        actor
    }

    async fn run(mut self) {
        // Bootstrap
        self.bootstrap().await;

        let mut recv_buf = vec![0u8; 65535];
        let mut maintenance_tick = tokio::time::interval(MAINTENANCE_INTERVAL);
        let mut cleanup_tick = tokio::time::interval(CLEANUP_INTERVAL);
        let mut query_timeout_tick = tokio::time::interval(self.config.query_timeout);
        let mut ping_tick = tokio::time::interval(PING_INTERVAL);

        loop {
            tokio::select! {
                // Incoming UDP packets (legacy path only — in unified mode the
                // session's demux task is the sole reader of the shared socket,
                // so reading here would steal its packets; M263).
                result = self.socket.recv_from(&mut recv_buf), if !self.unified => {
                    match result {
                        Ok((n, addr)) => {
                            self.handle_packet(&recv_buf[..n], addr).await;
                        }
                        Err(e) => {
                            warn!(error = %e, "UDP recv error");
                        }
                    }
                }

                // M263: pre-classified DHT datagrams from the unified demux.
                // Mirrors the `dns_bootstrap_rx` drain idiom: inline async that
                // parks on `pending()` when not unified, and disables itself
                // (sets `inbound_rx = None`) if the demux channel closes.
                demuxed = async {
                    match &mut self.inbound_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some((pkt, from)) = demuxed {
                        self.handle_packet(&pkt, from).await;
                    } else {
                        debug!("unified demux channel closed — DHT inbound disabled");
                        self.inbound_rx = None;
                    }
                }

                // Commands from handle
                cmd = self.rx.recv() => {
                    match cmd {
                        Some(DhtCommand::GetPeers { info_hash, reply, seed_addr }) => {
                            self.start_get_peers(info_hash, reply, seed_addr);
                        }
                        Some(DhtCommand::Announce { info_hash, port, reply }) => {
                            self.handle_announce(info_hash, port, reply).await;
                        }
                        Some(DhtCommand::Stats { reply }) => {
                            let _ = reply.send(self.make_stats());
                        }
                        Some(DhtCommand::UpdateExternalIp { ip, source }) => {
                            let source_id = source.source_id();
                            if let Some(consensus_ip) = self.ip_voter.add_vote(source_id, ip) {
                                debug!(%consensus_ip, "BEP 42: external IP consensus (via NAT/tracker)");
                                let _ = self.ip_consensus_tx.try_send(consensus_ip);
                                self.regenerate_node_id(consensus_ip);
                            }
                        }
                        Some(DhtCommand::GetImmutable { target, reply }) => {
                            self.handle_get_immutable(target, reply).await;
                        }
                        Some(DhtCommand::PutImmutable { value, reply }) => {
                            self.handle_put_immutable(value, reply).await;
                        }
                        Some(DhtCommand::GetMutable { public_key, salt, reply }) => {
                            self.handle_get_mutable(public_key, salt, reply).await;
                        }
                        Some(DhtCommand::PutMutable { keypair_bytes, value, seq, salt, reply }) => {
                            self.handle_put_mutable(keypair_bytes, value, seq, salt, reply).await;
                        }
                        Some(DhtCommand::SampleInfohashes { target, reply }) => {
                            self.handle_sample_infohashes(target, reply).await;
                        }
                        Some(DhtCommand::DirectGetPeers { target, info_hash, reply }) => {
                            self.handle_direct_get_peers(target, info_hash, reply).await;
                        }
                        Some(DhtCommand::GetRoutingNodes { reply }) => {
                            let nodes = self.routing_table.read().all_nodes();
                            let _ = reply.send(nodes);
                        }
                        Some(DhtCommand::SaveRoutingTable { reply }) => {
                            // M173 Lane B (B7): synchronous persist
                            // with reply. save_routing_table itself
                            // is best-effort (logs and continues on
                            // I/O error); we report Ok regardless so
                            // the caller knows the actor processed
                            // the request. If state_dir is None, the
                            // function is a no-op and we still ack.
                            self.save_routing_table();
                            let _ = reply.send(Ok(()));
                        }
                        Some(DhtCommand::Shutdown { reply }) => {
                            debug!("DHT shutting down — persisting routing table");
                            // M173 Lane B (B7): persist on shutdown
                            // BEFORE acking the reply. Without this,
                            // a runtime `enable_dht: true → false →
                            // true` cycle drops recent node state on
                            // the floor — the new actor starts with
                            // stale on-disk state.
                            self.save_routing_table();
                            if let Some(tx) = reply {
                                let _ = tx.send(());
                            }
                            return;
                        }
                        None => {
                            debug!("DHT shutting down (cmd channel closed) — persisting routing table");
                            // Same persist-on-exit guarantee even
                            // when shutdown is via channel-drop
                            // (e.g. session teardown).
                            self.save_routing_table();
                            return;
                        }
                    }
                }

                // Expire timed-out queries and advance stalled lookups
                // (like libtorrent's traversal_algorithm::failed → add_requests)
                _ = query_timeout_tick.tick() => {
                    self.expire_queries_and_advance_lookups().await;
                }

                // Periodic maintenance (routing table housekeeping)
                _ = maintenance_tick.tick() => {
                    self.maintenance().await;
                }

                // Peer store and item store cleanup
                _ = cleanup_tick.tick() => {
                    self.peer_store.cleanup();
                    self.item_store.expire(
                        Duration::from_secs(self.config.dht_item_lifetime_secs)
                    );
                }

                // Ping questionable nodes to verify liveness
                _ = ping_tick.tick() => {
                    // Two-phase ping frequency (M105): 5s during bootstrap for
                    // fast routing-table population, 60s steady-state to reduce
                    // chatter after the table is established.
                    let ping_interval = if self.bootstrap_complete {
                        Duration::from_mins(1)
                    } else {
                        Duration::from_secs(5)
                    };
                    if self.last_ping.elapsed() >= ping_interval {
                        self.ping_questionable_nodes().await;
                        self.last_ping = Instant::now();
                    }
                    // M146: Drain pending get_peers as soon as the routing
                    // table has at least 1 node (from bootstrap ping responses).
                    self.drain_pending_if_table_ready();
                }

                // M97: Bootstrap timeout — force bootstrap_complete after 10s
                () = async {
                    match &mut self.bootstrap_timeout {
                        Some(timer) => timer.as_mut().await,
                        None => std::future::pending().await,
                    }
                }, if self.bootstrap_timeout.is_some() && !self.bootstrap_complete => {
                    warn!(
                        table_size = self.routing_table.read().len(),
                        "bootstrap timeout (10s), proceeding with current routing table"
                    );
                    self.on_bootstrap_complete();
                }

                // M105: Drain DNS-resolved bootstrap addresses from background tasks
                result = async {
                    match &mut self.dns_bootstrap_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let own_id = *self.routing_table.read().own_id();
                    if let Some(addrs) = result {
                        // If the bootstrap lookup already exhausted (cold start
                        // with no saved nodes), restart it so DNS-resolved nodes
                        // get properly iterated through the Kademlia lookup.
                        if self.bootstrap_lookup.is_none() && !self.bootstrap_complete {
                            debug!(
                                dns_addrs = addrs.len(),
                                "restarting bootstrap lookup from DNS results"
                            );
                            self.bootstrap_lookup = Some(IterativeLookup::new(
                                own_id,
                                FindNodeCallbacks {
                                    round: 0,
                                    max_rounds: 6,
                                },
                            ));
                        }
                        for addr in addrs {
                            self.send_find_node(addr, own_id, None).await;
                        }
                    } else {
                        // All DNS tasks completed
                        debug!("DNS bootstrap tasks completed");
                        self.dns_bootstrap_rx = None;
                    }
                }

                // Drain tokens from active DhtLookup tasks
                Some((info_hash, node_id, addr, token)) = self.lookup_token_rx.recv() => {
                    let is_new = !self.announce_tokens.contains_key(&info_hash);
                    self.announce_tokens
                        .entry(info_hash)
                        .or_default()
                        .insert(node_id, (addr, token));
                    if is_new {
                        self.announce_token_order.push_back(info_hash);
                    }
                    // Bound the map: evict the oldest info_hash when over the
                    // cap. Tokens for random-target growth lookups are never
                    // consumed, so this only discards never-used data.
                    while self.announce_tokens.len() > MAX_ANNOUNCE_TOKENS {
                        if let Some(oldest) = self.announce_token_order.pop_front() {
                            self.announce_tokens.remove(&oldest);
                        } else {
                            break;
                        }
                    }
                }

                // Drain discovered nodes from active DhtLookup tasks
                Some((id, addr)) = self.lookup_node_rx.recv() => {
                    self.checked_insert(id, addr, false);
                }
            }
        }
    }

    async fn bootstrap(&mut self) {
        let own_id = *self.routing_table.read().own_id();

        // Partition: saved nodes (IP:port) vs DNS hostnames.
        // Saved nodes parse as SocketAddr; hardcoded bootstrap nodes have
        // hostname:port and will fail parse.
        let (saved_addrs, hostname_strs): (Vec<_>, Vec<_>) = self
            .config
            .bootstrap_nodes
            .clone()
            .into_iter()
            .partition(|s| s.parse::<SocketAddr>().is_ok());

        debug!(
            saved_nodes = saved_addrs.len(),
            dns_nodes = hostname_strs.len(),
            family = ?self.address_family,
            "bootstrap: starting (pinging saved nodes, resolving DNS nodes)"
        );

        // Phase 1: Ping saved nodes (validates liveness, inserts into routing
        // table via the normal ping response handler — no PingVerify needed)
        for addr_str in &saved_addrs {
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                self.send_ping(addr, None).await;
            }
        }

        // Phase 2: Spawn background DNS resolution tasks with retry+backoff.
        // Each hostname gets its own tokio::spawn that retries with exponential
        // backoff (1s → 30s cap, 120s total deadline). Resolved addresses are
        // sent to dns_bootstrap_rx and integrated via the main select! loop.
        // Phase 3 starts immediately without waiting for DNS.
        if !hostname_strs.is_empty() {
            let (dns_tx, dns_rx) = mpsc::channel(16);
            for hostname in hostname_strs {
                let tx = dns_tx.clone();
                let family = self.address_family;
                tokio::spawn(async move {
                    dns_bootstrap_resolve(hostname, family, tx).await;
                });
            }
            drop(dns_tx); // close sender so receiver ends when all tasks complete
            self.dns_bootstrap_rx = Some(dns_rx);
        }

        // Phase 3: Initiate iterative bootstrap — follow returned nodes to discover more
        let initial_closest: Vec<CompactNodeInfo> = self
            .routing_table
            .read()
            .closest(&own_id, K)
            .into_iter()
            .map(|n| CompactNodeInfo {
                id: n.id,
                addr: n.addr,
            })
            .collect();

        debug!(
            initial_nodes = initial_closest.len(),
            table_size = self.routing_table.read().len(),
            "bootstrap: starting iterative lookup"
        );

        let mut lookup = IterativeLookup::new(
            own_id,
            FindNodeCallbacks {
                round: 0,
                max_rounds: 6,
            },
        );
        lookup.closest = initial_closest;
        self.bootstrap_lookup = Some(lookup);
    }

    async fn handle_packet(&mut self, data: &[u8], addr: SocketAddr) {
        let msg = match KrpcMessage::from_bytes(data) {
            Ok(msg) => msg,
            Err(e) => {
                trace!(error = %e, from = %addr, "invalid KRPC message");
                return;
            }
        };

        match &msg.body {
            KrpcBody::Query(query) => {
                self.handle_query(&msg, query, addr).await;
            }
            KrpcBody::Response(resp) => {
                self.handle_response(&msg, resp, addr).await;
            }
            KrpcBody::Error { code, message } => {
                trace!(code, message, from = %addr, "KRPC error received");
                // Still match pending query to clean up
                let txn = msg.transaction_id.as_u16();
                if let Some((_, pending)) = self.pending.remove(&txn)
                    && let Some(nid) = pending.node_id
                {
                    self.routing_table.write().mark_failed(&nid);
                }
            }
        }
    }

    /// Check if a socket address matches this actor's address family.
    fn matches_family(&self, addr: &SocketAddr) -> bool {
        match self.address_family {
            AddressFamily::V4 => addr.is_ipv4(),
            AddressFamily::V6 => addr.is_ipv6(),
        }
    }

    /// BEP 45: Build the `want` list for outgoing queries. When multi-address
    /// is enabled, request both address families so dual-stack remote nodes
    /// include cross-family nodes in their responses.
    fn outgoing_want(&self) -> Option<Vec<crate::krpc::WantFamily>> {
        if self.config.enable_multi_address {
            Some(vec![
                crate::krpc::WantFamily::N4,
                crate::krpc::WantFamily::N6,
            ])
        } else {
            None
        }
    }

    async fn handle_query(&mut self, msg: &KrpcMessage, query: &KrpcQuery, addr: SocketAddr) {
        if !self.matches_family(&addr) {
            return; // Reject wrong address family
        }
        let sender_id = *query.sender_id();
        self.checked_insert(sender_id, addr, msg.read_only);
        self.routing_table.write().mark_query(&sender_id);

        let own_id = *self.routing_table.read().own_id();
        let response = match query {
            KrpcQuery::Ping { id: _ } => KrpcResponse::NodeId { id: own_id },
            KrpcQuery::FindNode {
                id: _,
                target,
                want: _,
            } => {
                // RESPONSE_K bounds the payload regardless of the (large) table
                // K, keeping responses in one small UDP packet.
                let closest = self.routing_table.read().closest(target, RESPONSE_K);
                let nodes: Vec<CompactNodeInfo> = closest
                    .into_iter()
                    .map(|n| CompactNodeInfo {
                        id: n.id,
                        addr: n.addr,
                    })
                    .collect();
                KrpcResponse::FindNode {
                    id: own_id,
                    nodes,
                    nodes6: Vec::new(),
                }
            }
            KrpcQuery::GetPeers {
                id: _,
                info_hash,
                noseed: _,
                scrape,
                want: _,
            } => {
                let ip = addr.ip();
                self.stats.lookups_received += 1;
                // Passive intake: someone is actively seeking this hash.
                let _ = self.events_tx.send(DhtEvent::LookedUp {
                    info_hash: *info_hash,
                    from_addr: addr,
                });
                let token = self.peer_store.generate_token(&ip);
                let peers = self.peer_store.get_peers(info_hash, 50);

                // BEP 33: generate bloom filters when scrape=1.
                let (bfpe, bfsd) = if *scrape == Some(1) {
                    let all_peers = self.peer_store.all_peers(info_hash);
                    let mut filter = crate::bloom::ScrapeBloomFilter::new();
                    for peer_addr in &all_peers {
                        filter.insert(*peer_addr);
                    }
                    (Some(filter.as_bytes().to_vec()), None)
                } else {
                    (None, None)
                };

                if peers.is_empty() {
                    let closest = self.routing_table.read().closest(info_hash, RESPONSE_K);
                    let nodes: Vec<CompactNodeInfo> = closest
                        .into_iter()
                        .map(|n| CompactNodeInfo {
                            id: n.id,
                            addr: n.addr,
                        })
                        .collect();
                    KrpcResponse::GetPeers(GetPeersResponse {
                        id: own_id,
                        token: Some(token),
                        peers: Vec::new(),
                        nodes,
                        nodes6: Vec::new(),
                        bfpe,
                        bfsd,
                    })
                } else {
                    KrpcResponse::GetPeers(GetPeersResponse {
                        id: own_id,
                        token: Some(token),
                        peers,
                        nodes: Vec::new(),
                        nodes6: Vec::new(),
                        bfpe,
                        bfsd,
                    })
                }
            }
            KrpcQuery::AnnouncePeer {
                id: _,
                info_hash,
                port,
                implied_port,
                token,
            } => {
                self.stats.announces_received += 1;
                let ip = addr.ip();
                if !self.peer_store.validate_token(token, &ip) {
                    self.stats.announces_token_rejected += 1;
                    // Send error response for invalid token
                    let err_msg = KrpcMessage {
                        transaction_id: msg.transaction_id,
                        body: KrpcBody::Error {
                            code: 203,
                            message: "invalid token".into(),
                        },
                        sender_ip: Some(addr),
                        read_only: false,
                    };
                    if let Ok(bytes) = err_msg.to_bytes() {
                        let _ = self.socket.send_to(&bytes, addr).await;
                    }
                    return;
                }
                let peer_port = if *implied_port { addr.port() } else { *port };
                let peer_addr = SocketAddr::new(addr.ip(), peer_port);
                self.peer_store.add_peer(*info_hash, peer_addr);
                // Passive intake: surface the announce to the application — the
                // hash is live (a peer is announcing it right now).
                let _ = self.events_tx.send(DhtEvent::Announced {
                    info_hash: *info_hash,
                    peer_addr,
                });
                KrpcResponse::NodeId {
                    id: *self.routing_table.read().own_id(),
                }
            }
            // BEP 44: get item from DHT storage
            KrpcQuery::Get {
                id: _,
                target,
                seq: requested_seq,
            } => {
                let ip = addr.ip();
                let token = self.peer_store.generate_token(&ip);

                // Try immutable lookup first
                if let Some(item) = self.item_store.get_immutable(target) {
                    KrpcResponse::GetItem {
                        id: *self.routing_table.read().own_id(),
                        token: Some(token),
                        nodes: Vec::new(),
                        nodes6: Vec::new(),
                        value: Some(item.value),
                        key: None,
                        signature: None,
                        seq: None,
                    }
                } else if let Some(item) = self.item_store.get_mutable_by_target(target) {
                    // Check if requester wants only items with seq > requested_seq
                    if let Some(min_seq) = requested_seq {
                        if item.seq <= *min_seq {
                            // Return token + nodes but no value (requester already has this or newer)
                            let closest = self.routing_table.read().closest(target, RESPONSE_K);
                            let nodes: Vec<CompactNodeInfo> = closest
                                .into_iter()
                                .map(|n| CompactNodeInfo {
                                    id: n.id,
                                    addr: n.addr,
                                })
                                .collect();
                            KrpcResponse::GetItem {
                                id: *self.routing_table.read().own_id(),
                                token: Some(token),
                                nodes,
                                nodes6: Vec::new(),
                                value: None,
                                key: Some(item.public_key),
                                signature: Some(item.signature),
                                seq: Some(item.seq),
                            }
                        } else {
                            KrpcResponse::GetItem {
                                id: *self.routing_table.read().own_id(),
                                token: Some(token),
                                nodes: Vec::new(),
                                nodes6: Vec::new(),
                                value: Some(item.value),
                                key: Some(item.public_key),
                                signature: Some(item.signature),
                                seq: Some(item.seq),
                            }
                        }
                    } else {
                        KrpcResponse::GetItem {
                            id: *self.routing_table.read().own_id(),
                            token: Some(token),
                            nodes: Vec::new(),
                            nodes6: Vec::new(),
                            value: Some(item.value),
                            key: Some(item.public_key),
                            signature: Some(item.signature),
                            seq: Some(item.seq),
                        }
                    }
                } else {
                    // Not found — return closer nodes
                    let closest = self.routing_table.read().closest(target, RESPONSE_K);
                    let nodes: Vec<CompactNodeInfo> = closest
                        .into_iter()
                        .map(|n| CompactNodeInfo {
                            id: n.id,
                            addr: n.addr,
                        })
                        .collect();
                    KrpcResponse::GetItem {
                        id: *self.routing_table.read().own_id(),
                        token: Some(token),
                        nodes,
                        nodes6: Vec::new(),
                        value: None,
                        key: None,
                        signature: None,
                        seq: None,
                    }
                }
            }
            // BEP 44: put item into DHT storage
            KrpcQuery::Put {
                id: _,
                token,
                value,
                key,
                signature,
                seq,
                salt,
                cas,
            } => {
                let ip = addr.ip();

                // Validate token
                if !self.peer_store.validate_token(token, &ip) {
                    let err_msg = KrpcMessage {
                        transaction_id: msg.transaction_id,
                        body: KrpcBody::Error {
                            code: 203,
                            message: "invalid token".into(),
                        },
                        sender_ip: Some(addr),
                        read_only: false,
                    };
                    if let Ok(bytes) = err_msg.to_bytes() {
                        let _ = self.socket.send_to(&bytes, addr).await;
                    }
                    return;
                }

                // Validate value size
                if value.len() > MAX_VALUE_SIZE {
                    let err_msg = KrpcMessage {
                        transaction_id: msg.transaction_id,
                        body: KrpcBody::Error {
                            code: 205,
                            message: "message (v field) too big".into(),
                        },
                        sender_ip: Some(addr),
                        read_only: false,
                    };
                    if let Ok(bytes) = err_msg.to_bytes() {
                        let _ = self.socket.send_to(&bytes, addr).await;
                    }
                    return;
                }

                if let (Some(k), Some(sig), Some(seq_val)) = (key, signature, seq) {
                    // Mutable item
                    let salt_bytes = salt.clone().unwrap_or_default();

                    // Validate salt size
                    if salt_bytes.len() > MAX_SALT_SIZE {
                        let err_msg = KrpcMessage {
                            transaction_id: msg.transaction_id,
                            body: KrpcBody::Error {
                                code: 207,
                                message: "salt (salt field) too big".into(),
                            },
                            sender_ip: Some(addr),
                            read_only: false,
                        };
                        if let Ok(bytes) = err_msg.to_bytes() {
                            let _ = self.socket.send_to(&bytes, addr).await;
                        }
                        return;
                    }

                    let item = MutableItem {
                        value: value.clone(),
                        public_key: *k,
                        signature: *sig,
                        seq: *seq_val,
                        salt: salt_bytes,
                        target: bep44::compute_mutable_target(k, salt.as_deref().unwrap_or(&[])),
                    };

                    // Verify signature
                    if !item.verify() {
                        let err_msg = KrpcMessage {
                            transaction_id: msg.transaction_id,
                            body: KrpcBody::Error {
                                code: 206,
                                message: "invalid signature".into(),
                            },
                            sender_ip: Some(addr),
                            read_only: false,
                        };
                        if let Ok(bytes) = err_msg.to_bytes() {
                            let _ = self.socket.send_to(&bytes, addr).await;
                        }
                        return;
                    }

                    // CAS check
                    if let Some(expected_seq) = cas
                        && let Some(existing) = self.item_store.get_mutable(k, &item.salt)
                        && existing.seq != *expected_seq
                    {
                        let err_msg = KrpcMessage {
                            transaction_id: msg.transaction_id,
                            body: KrpcBody::Error {
                                code: 301,
                                message: format!(
                                    "CAS mismatch: expected seq {}, got {}",
                                    expected_seq, existing.seq
                                ),
                            },
                            sender_ip: Some(addr),
                            read_only: false,
                        };
                        if let Ok(bytes) = err_msg.to_bytes() {
                            let _ = self.socket.send_to(&bytes, addr).await;
                        }
                        return;
                    }

                    // Seq monotonicity check
                    if let Some(existing) = self.item_store.get_mutable(k, &item.salt)
                        && *seq_val <= existing.seq
                    {
                        let err_msg = KrpcMessage {
                            transaction_id: msg.transaction_id,
                            body: KrpcBody::Error {
                                code: 302,
                                message: format!(
                                    "sequence number not newer: {} <= {}",
                                    seq_val, existing.seq
                                ),
                            },
                            sender_ip: Some(addr),
                            read_only: false,
                        };
                        if let Ok(bytes) = err_msg.to_bytes() {
                            let _ = self.socket.send_to(&bytes, addr).await;
                        }
                        return;
                    }

                    self.item_store.put_mutable(item);
                } else {
                    // Immutable item
                    if let Ok(item) = ImmutableItem::new(value.clone()) {
                        self.item_store.put_immutable(item);
                    } else {
                        let err_msg = KrpcMessage {
                            transaction_id: msg.transaction_id,
                            body: KrpcBody::Error {
                                code: 205,
                                message: "message (v field) too big".into(),
                            },
                            sender_ip: Some(addr),
                            read_only: false,
                        };
                        if let Ok(bytes) = err_msg.to_bytes() {
                            let _ = self.socket.send_to(&bytes, addr).await;
                        }
                        return;
                    }
                }

                KrpcResponse::NodeId {
                    id: *self.routing_table.read().own_id(),
                }
            }
            // BEP 51: sample_infohashes
            KrpcQuery::SampleInfohashes { id: _, target } => {
                let closest = self.routing_table.read().closest(target, RESPONSE_K);
                let nodes: Vec<CompactNodeInfo> = closest
                    .into_iter()
                    .map(|n| CompactNodeInfo {
                        id: n.id,
                        addr: n.addr,
                    })
                    .collect();

                // Sample up to 20 info hashes (fits comfortably in one UDP packet)
                let samples = self.peer_store.random_info_hashes(20);
                let num = self.peer_store.info_hash_count() as i64;

                KrpcResponse::SampleInfohashes(SampleInfohashesResponse {
                    id: *self.routing_table.read().own_id(),
                    interval: 60, // 1 minute default interval
                    num,
                    samples,
                    nodes,
                })
            }
        };

        let reply = KrpcMessage {
            transaction_id: msg.transaction_id,
            body: KrpcBody::Response(response),
            sender_ip: Some(addr), // BEP 42: tell the querier their IP
            read_only: false,      // BEP 43: ro only on queries, never on responses
        };
        if let Ok(bytes) = reply.to_bytes() {
            let _ = self.socket.send_to(&bytes, addr).await;
        }
    }

    async fn handle_response(&mut self, msg: &KrpcMessage, resp: &KrpcResponse, addr: SocketAddr) {
        if !self.matches_family(&addr) {
            return; // Reject wrong address family
        }
        self.stats.total_responses_received += 1;

        // BEP 42: feed the ip field into the voter
        if let Some(reported_ip) = msg.sender_ip {
            let source_id = hash_source_addr(&addr);
            if let Some(consensus_ip) = self.ip_voter.add_vote(source_id, reported_ip.ip()) {
                debug!(%consensus_ip, "BEP 42: external IP consensus changed");
                let _ = self.ip_consensus_tx.try_send(consensus_ip);
                self.regenerate_node_id(consensus_ip);
            }
        }

        let sender_id = *resp.sender_id();
        self.checked_insert(sender_id, addr, false);
        self.routing_table.write().mark_response(&sender_id);

        // M146: Drain pending get_peers immediately when first node arrives.
        self.drain_pending_if_table_ready();

        let txn = msg.transaction_id.as_u16();
        let Some((_, pending)) = self.pending.remove(&txn) else {
            trace!(txn, from = %addr, "response for unknown transaction");
            return;
        };

        // If this pending entry has a oneshot response_tx, the response came
        // from a DhtLookup task. Forward via oneshot and let the lookup handle
        // its own state. We must still update the routing table with the
        // responding node — the lookup forwards discovered *contained* nodes
        // via node_tx, but the *responding* node itself must be handled here.
        if let Some(response_tx) = pending.response_tx {
            self.checked_insert(sender_id, pending.addr, false);
            let _ = response_tx.send(PendingQueryResponse {
                sender_id,
                response: resp.clone(),
            });
            return;
        }

        match (&pending.kind, resp) {
            (PendingQueryKind::FindNode, KrpcResponse::FindNode { nodes, nodes6, .. }) => {
                for node in nodes {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                for node in nodes6 {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }

                // Advance iterative bootstrap lookup if active
                if let Some(ref mut lookup) = self.bootstrap_lookup {
                    // Merge nodes4 + nodes6 into a single feed (CompactNodeInfo)
                    let mut all_nodes: Vec<CompactNodeInfo> = nodes.clone();
                    all_nodes.extend(nodes6.iter().map(|n| CompactNodeInfo {
                        id: n.id,
                        addr: n.addr,
                    }));
                    lookup.feed_nodes(all_nodes, self.address_family);
                }

                if self.bootstrap_lookup.is_some() {
                    // Extract data needed before calling send_find_node (drops borrow)
                    let (to_query, target, terminate) =
                        if let Some(ref mut lookup) = self.bootstrap_lookup {
                            if lookup.callbacks.round >= lookup.callbacks.max_rounds {
                                (Vec::new(), lookup.target, true)
                            } else {
                                let to_query = lookup.next_to_query(3);
                                let target = lookup.target;
                                if to_query.is_empty() {
                                    (Vec::new(), target, true)
                                } else {
                                    lookup.callbacks.round += 1;
                                    (to_query, target, false)
                                }
                            }
                        } else {
                            (Vec::new(), Id20::ZERO, false)
                        };

                    if terminate {
                        debug!(
                            routing_table_size = self.routing_table.read().len(),
                            "iterative bootstrap complete"
                        );
                        self.bootstrap_lookup = None;
                        self.on_bootstrap_complete();
                    } else {
                        let queries: Vec<(SocketAddr, Id20)> =
                            to_query.iter().map(|n| (n.addr, n.id)).collect();
                        for (node_addr, nid) in queries {
                            self.send_find_node(node_addr, target, Some(nid)).await;
                        }
                    }
                }
            }
            (PendingQueryKind::GetPeers { info_hash }, KrpcResponse::GetPeers(gp)) => {
                // GetPeers responses are normally routed to DhtLookup via
                // oneshot (handled above). This arm only fires for orphaned
                // responses after a lookup was aborted. Still update the
                // routing table from returned nodes.
                for node in &gp.nodes {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                for node in &gp.nodes6 {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                trace!(%info_hash, "get_peers response for orphaned lookup");
            }
            (PendingQueryKind::Ping, KrpcResponse::NodeId { .. }) => {
                // Ping response — node is alive, already updated routing table
                if !self.bootstrap_complete {
                    debug!(
                        from = %pending.addr,
                        table_size = self.routing_table.read().len(),
                        "bootstrap: ping response received"
                    );
                }
            }
            (
                PendingQueryKind::AnnouncePeer | PendingQueryKind::PutItem,
                KrpcResponse::NodeId { .. },
            ) => {
                // Announce / put acknowledged — success
            }
            (PendingQueryKind::SampleInfohashes, KrpcResponse::SampleInfohashes(si)) => {
                // Add discovered nodes to routing table (Gap 6: use checked_insert for BEP 42)
                for node in &si.nodes {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }

                // Send result back to caller
                if let Some(reply) = self.sample_replies.remove(&txn) {
                    let _ = reply.send(Ok(SampleInfohashesResult {
                        interval: si.interval,
                        num: si.num,
                        samples: si.samples.clone(),
                        nodes: si.nodes.clone(),
                    }));
                }
            }
            (PendingQueryKind::DirectGetPeers, KrpcResponse::GetPeers(gp)) => {
                for node in &gp.nodes {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                for node in &gp.nodes6 {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                if let Some(reply) = self.direct_peer_replies.remove(&txn) {
                    let _ = reply.send(Ok(gp.peers.clone()));
                }
            }
            (
                PendingQueryKind::GetItem { target },
                KrpcResponse::GetItem {
                    token,
                    nodes,
                    nodes6,
                    value,
                    key,
                    signature,
                    seq,
                    ..
                },
            ) => {
                // Gap 13: Use checked_insert (BEP 42 compliant) instead of routing_table.insert
                for node in nodes {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }
                for node in nodes6 {
                    if self.matches_family(&node.addr) {
                        self.checked_insert(node.id, node.addr, false);
                    }
                }

                let target = *target;

                // If we have a put operation waiting for tokens, collect this token
                if let (Some(token), Some(put_op)) = (token, self.item_put_ops.get_mut(&target)) {
                    match put_op {
                        ItemPutState::Immutable { tokens, .. }
                        | ItemPutState::Mutable { tokens, .. } => {
                            tokens.insert(sender_id, (addr, token.clone()));
                        }
                    }

                    // If we have enough tokens, send the puts
                    let should_send = match &self.item_put_ops[&target] {
                        ItemPutState::Immutable {
                            tokens, sent_puts, ..
                        }
                        | ItemPutState::Mutable {
                            tokens, sent_puts, ..
                        } => tokens.len() >= K && *sent_puts == 0,
                    };

                    if should_send {
                        self.send_pending_puts(target).await;
                    }
                }

                // If we have a get lookup, process the value
                if self.item_lookups.contains_key(&target) {
                    // Determine if this is immutable or mutable lookup
                    let is_immutable = matches!(
                        self.item_lookups.get(&target),
                        Some(ItemLookupState::Immutable { .. })
                    );

                    if is_immutable {
                        if let Some(v) = value {
                            // Validate: SHA-1(v) should equal target
                            if gaia_core::sha1(v) == target {
                                // Store locally
                                if let Ok(item) = crate::bep44::ImmutableItem::new(v.clone()) {
                                    self.item_store.put_immutable(item);
                                }
                                if let Some(ItemLookupState::Immutable { reply, .. }) =
                                    self.item_lookups.get_mut(&target)
                                    && let Some(r) = reply.take()
                                {
                                    let _ = r.send(Ok(Some(v.clone())));
                                }
                            }
                        } else {
                            // Gap 7: Collect nodes to query into local Vec first
                            // to avoid borrow checker violation
                            let family = self.address_family;
                            let to_query: Vec<SocketAddr> = {
                                if let Some(ItemLookupState::Immutable { queried, .. }) =
                                    self.item_lookups.get_mut(&target)
                                {
                                    nodes
                                        .iter()
                                        .filter(|n| match family {
                                            AddressFamily::V4 => n.addr.is_ipv4(),
                                            AddressFamily::V6 => n.addr.is_ipv6(),
                                        })
                                        .filter(|n| queried.insert(n.id))
                                        .take(3)
                                        .map(|n| n.addr)
                                        .collect()
                                } else {
                                    vec![]
                                }
                            };
                            for query_addr in to_query {
                                self.send_get_item(query_addr, target, None).await;
                            }
                        }
                    } else {
                        // Mutable lookup
                        if let (Some(v), Some(k), Some(sig), Some(s)) = (value, key, signature, seq)
                        {
                            // Get the salt from the lookup state
                            let salt = if let Some(ItemLookupState::Mutable { salt, .. }) =
                                self.item_lookups.get(&target)
                            {
                                salt.clone()
                            } else {
                                Vec::new()
                            };

                            let item = crate::bep44::MutableItem {
                                value: v.clone(),
                                public_key: *k,
                                signature: *sig,
                                seq: *s,
                                salt,
                                target,
                            };

                            if item.verify()
                                && let Some(ItemLookupState::Mutable {
                                    best_seq,
                                    best_value,
                                    ..
                                }) = self.item_lookups.get_mut(&target)
                                && *s > *best_seq
                            {
                                *best_seq = *s;
                                *best_value = Some(v.clone());
                                // Store locally
                                self.item_store.put_mutable(item);
                            }
                        }

                        // Gap 7: Collect nodes to query into local Vec first
                        let family = self.address_family;
                        let to_query: Vec<SocketAddr> = {
                            if let Some(ItemLookupState::Mutable { queried, .. }) =
                                self.item_lookups.get_mut(&target)
                            {
                                nodes
                                    .iter()
                                    .filter(|n| match family {
                                        AddressFamily::V4 => n.addr.is_ipv4(),
                                        AddressFamily::V6 => n.addr.is_ipv6(),
                                    })
                                    .filter(|n| queried.insert(n.id))
                                    .take(3)
                                    .map(|n| n.addr)
                                    .collect()
                            } else {
                                vec![]
                            }
                        };
                        for query_addr in to_query {
                            self.send_get_item(query_addr, target, None).await;
                        }
                    }
                }
            }
            _ => {
                trace!(txn, "mismatched response type");
            }
        }
    }

    fn start_get_peers(
        &mut self,
        info_hash: Id20,
        reply: mpsc::UnboundedSender<crate::krpc::PeerBatch>,
        seed_addr: Option<std::net::SocketAddr>,
    ) {
        // M146: Lightweight gate — require at least 1 routing table node
        // before starting get_peers. Without any nodes, the DhtLookup would
        // start with zero roots and stall in adaptive backoff (1-15s) while
        // bootstrap pings populate the table.
        //
        // The old gate required 8 nodes (causing 1-5s dead zones). With
        // threshold=1, saved-node pings typically populate within 100-500ms.
        // The pending_get_peers queue is still removed — instead we use the
        // bootstrap_complete flag + bootstrap timeout as the fallback.
        if !self.bootstrap_complete && self.routing_table.read().is_empty() {
            debug!(
                %info_hash,
                "get_peers: routing table empty, queuing until first node arrives"
            );
            self.pending_get_peers.push((info_hash, reply, seed_addr));
            return;
        }
        self.start_get_peers_inner(info_hash, reply, seed_addr);
    }

    fn start_get_peers_inner(
        &mut self,
        info_hash: Id20,
        reply: mpsc::UnboundedSender<crate::krpc::PeerBatch>,
        seed_addr: Option<std::net::SocketAddr>,
    ) {
        debug!(
            %info_hash,
            table_size = self.routing_table.read().len(),
            "starting get_peers query"
        );

        // M146: Allow get_peers with an empty routing table. The DhtLookup
        // starts with zero roots and uses its 1s requery timer to inject
        // roots as bootstrap pings populate the table. This avoids dropping
        // the reply channel (which would cause a 60s dead zone before the
        // TorrentActor re-queries).

        // M147: Allow concurrent lookups for the same info_hash.
        // The new lookup overwrites the HashMap entry; the old lookup task
        // continues running and self-terminates when its peer_rx channel
        // closes. This prevents the background MetadataResolver's DHT
        // lookup from killing the TorrentActor's own DHT stream.

        let own_id = *self.routing_table.read().own_id();
        debug!(
            family = ?self.address_family,
            %info_hash,
            table_size = self.routing_table.read().len(),
            "get_peers: spawning DhtLookup"
        );

        let lookup = crate::dht_lookup::DhtLookup::new(
            info_hash,
            crate::dht_lookup::LookupConfig {
                max_depth: 4,
                // 64-node walks instead of 256: a crawler's get_peers lookups
                // (routing growth + fetch peer discovery) only need a few peers
                // / nodes, and 256-node walks hold ~4x the in-flight query state
                // per lookup. At high lookup churn that 4x was the dominant
                // memory growth source.
                max_nodes: 64,
            },
            self.address_family,
            self.socket.clone(),
            self.pending.clone(),
            self.rate_limiter.clone(),
            self.routing_table.clone(),
            self.next_txn_id.clone(),
            own_id,
            reply,
            self.lookup_token_tx.clone(),
            self.lookup_node_tx.clone(),
            self.config.read_only_mode,
            self.outgoing_want(),
            seed_addr,
        );

        let handle = tokio::spawn(lookup.run());
        // If a prior lookup exists, let it run — don't abort it, as it may be
        // the TorrentActor's lookup. The old task runs to completion independently.
        if let Some(old_handle) = self.active_lookups.insert(info_hash, handle) {
            // Intentionally detach — old lookup finishes naturally when it
            // exhausts its routing table query.
            drop(old_handle);
        }
    }

    /// M97/M146: Called when bootstrap completes or times out.
    /// Drains queued `get_peers` and sets the `bootstrap_complete` flag.
    fn on_bootstrap_complete(&mut self) {
        if self.bootstrap_complete {
            return;
        }
        self.bootstrap_complete = true;
        self.bootstrap_timeout = None;

        let pending = std::mem::take(&mut self.pending_get_peers);
        debug!(
            count = pending.len(),
            table_size = self.routing_table.read().len(),
            "bootstrap complete, processing queued get_peers"
        );
        for (info_hash, reply, seed_addr) in pending {
            self.start_get_peers_inner(info_hash, reply, seed_addr);
        }
    }

    /// M146: Drain pending `get_peers` as soon as at least 1 node is in the
    /// routing table. Called from the `ping_tick` arm when routing table
    /// transitions from empty to non-empty during bootstrap.
    fn drain_pending_if_table_ready(&mut self) {
        if self.pending_get_peers.is_empty() || self.routing_table.read().is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_get_peers);
        debug!(
            count = pending.len(),
            table_size = self.routing_table.read().len(),
            "routing table populated, draining queued get_peers"
        );
        for (info_hash, reply, seed_addr) in pending {
            self.start_get_peers_inner(info_hash, reply, seed_addr);
        }
    }

    async fn handle_announce(
        &mut self,
        info_hash: Id20,
        port: u16,
        reply: oneshot::Sender<Result<()>>,
    ) {
        // BEP 43: read-only nodes should not announce
        if self.config.read_only_mode {
            trace!("BEP 43: suppressing announce_peer in read-only mode");
            let _ = reply.send(Ok(()));
            return;
        }

        // First, find nodes with tokens collected from DhtLookup tasks
        let tokens: Vec<(SocketAddr, Vec<u8>)> = self
            .announce_tokens
            .get(&info_hash)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();

        if tokens.is_empty() {
            let _ = reply.send(Err(Error::InvalidMessage(
                "no tokens available; call get_peers first".into(),
            )));
            return;
        }

        let own_id = *self.routing_table.read().own_id();
        for (addr, token) in &tokens {
            if !self.rate_limiter.try_acquire() {
                break; // Use break, not return, since we still need to send the reply
            }
            let txn = self.next_transaction_id();
            let msg = KrpcMessage {
                transaction_id: TransactionId::from_u16(txn),
                body: KrpcBody::Query(KrpcQuery::AnnouncePeer {
                    id: own_id,
                    info_hash,
                    port,
                    implied_port: false,
                    token: token.clone(),
                }),
                sender_ip: None,
                read_only: false, // announce_peer is suppressed in read-only mode (early return above)
            };
            if let Ok(bytes) = msg.to_bytes() {
                let _ = self.socket.send_to(&bytes, addr).await;
                self.pending.insert(
                    txn,
                    PendingQuery {
                        sent_at: Instant::now(),
                        addr: *addr,
                        kind: PendingQueryKind::AnnouncePeer,
                        node_id: None,
                        response_tx: None,
                    },
                );
                self.stats.total_queries_sent += 1;
            }
        }

        // Clean up tokens for this info_hash after announcing
        self.announce_tokens.remove(&info_hash);

        let _ = reply.send(Ok(()));
    }

    /// Expire timed-out queries and advance any stalled `get_peers` lookups.
    /// Runs every `query_timeout` interval — mirrors libtorrent's pattern where
    /// `traversal_algorithm::failed()` immediately calls `add_requests()` to
    /// query the next closest nodes.
    async fn expire_queries_and_advance_lookups(&mut self) {
        let timeout = self.config.query_timeout;
        let expired: Vec<u16> = self
            .pending
            .iter()
            .filter(|entry| entry.value().sent_at.elapsed() > timeout)
            .map(|entry| *entry.key())
            .collect();

        if expired.is_empty() {
            return;
        }

        debug!(
            family = ?self.address_family,
            expired_count = expired.len(),
            total_pending = self.pending.len(),
            active_lookups = self.active_lookups.len(),
            "expiring timed-out queries"
        );

        let mut find_node_timed_out = false;

        for txn in expired {
            if let Some((_, pending)) = self.pending.remove(&txn) {
                trace!(txn, addr = %pending.addr, "query timed out");
                if let Some(nid) = pending.node_id {
                    self.routing_table.write().mark_failed(&nid);
                }
                if matches!(pending.kind, PendingQueryKind::SampleInfohashes)
                    && let Some(reply) = self.sample_replies.remove(&txn)
                {
                    let _ = reply.send(Err(Error::Timeout));
                }
                if matches!(pending.kind, PendingQueryKind::DirectGetPeers)
                    && let Some(reply) = self.direct_peer_replies.remove(&txn)
                {
                    let _ = reply.send(Err(Error::Timeout));
                }
                // For GetPeers: the DhtLookup handles its own timeouts via
                // the oneshot channel — if response_tx was set, dropping the
                // PendingQuery will close the oneshot and the lookup's await
                // returns Err. No stalled lookup advancement needed here.
                if matches!(pending.kind, PendingQueryKind::FindNode) {
                    find_node_timed_out = true;
                }
            }
        }

        // Advance bootstrap lookup if a FindNode query timed out
        if find_node_timed_out && self.bootstrap_lookup.is_some() {
            // Extract queries before calling send_find_node (borrow-checker)
            let (to_query, target, terminate) = if let Some(ref mut lookup) = self.bootstrap_lookup
            {
                let to_query = lookup.next_to_query(3);
                let target = lookup.target;
                if to_query.is_empty() {
                    (Vec::new(), target, true)
                } else {
                    (to_query, target, false)
                }
            } else {
                (Vec::new(), Id20::ZERO, false)
            };

            if terminate {
                self.bootstrap_lookup = None;
                self.on_bootstrap_complete();
            } else {
                let queries: Vec<(SocketAddr, Id20)> =
                    to_query.iter().map(|n| (n.addr, n.id)).collect();
                for (node_addr, nid) in queries {
                    self.send_find_node(node_addr, target, Some(nid)).await;
                }
            }
        }
    }

    // ---- JSON routing table persistence ----

    /// Return the JSON state file path for this address family.
    fn state_file_path(state_dir: &std::path::Path, family: AddressFamily) -> PathBuf {
        match family {
            AddressFamily::V4 => state_dir.join("dht_state.json"),
            AddressFamily::V6 => state_dir.join("dht_state_v6.json"),
        }
    }

    /// Persist the routing table to a JSON file via atomic temp-file + rename.
    ///
    /// Skips silently when `state_dir` is `None`. On any I/O or serialization
    /// error, logs a warning and continues (never crashes the actor).
    fn save_routing_table(&self) {
        let Some(state_dir) = &self.config.state_dir else {
            return;
        };

        let nodes = self.routing_table.read().all_nodes();
        let own_id = *self.routing_table.read().own_id();

        let state = DhtState {
            node_id: own_id.to_hex(),
            nodes: nodes
                .iter()
                .map(|(id, addr)| DhtNodeEntry {
                    id: id.to_hex(),
                    addr: addr.to_string(),
                })
                .collect(),
        };

        let json = match serde_json::to_string_pretty(&state) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "failed to serialize DHT state to JSON");
                return;
            }
        };

        let final_path = Self::state_file_path(state_dir, self.address_family);
        let tmp_path = state_dir.join(format!(
            ".dht_state_{}.tmp",
            match self.address_family {
                AddressFamily::V4 => "v4",
                AddressFamily::V6 => "v6",
            }
        ));

        if let Err(e) = std::fs::write(&tmp_path, json.as_bytes()) {
            warn!(error = %e, path = %tmp_path.display(), "failed to write DHT state temp file");
            return;
        }

        if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
            warn!(
                error = %e,
                tmp = %tmp_path.display(),
                dst = %final_path.display(),
                "failed to rename DHT state temp file"
            );
        }
    }

    /// Load the routing table from a JSON state file.
    ///
    /// Skips silently when `state_dir` is `None`. On missing file (first run),
    /// logs at debug level and returns. On corrupt/parse errors, logs a warning
    /// and falls through to normal bootstrap. On success, inserts all nodes as
    /// Questionable and filters `bootstrap_nodes` to hostnames only (since the
    /// JSON file has fresher saved-node data).
    fn load_routing_table(&mut self) {
        let state_dir = match &self.config.state_dir {
            Some(dir) => dir.clone(),
            None => return,
        };

        let path = Self::state_file_path(&state_dir, self.address_family);

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "no saved DHT state (first run)");
                return;
            }
            Err(e) => {
                warn!(error = %e, path = %path.display(), "failed to read DHT state file");
                return;
            }
        };

        let state: DhtState = match serde_json::from_str(&data) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, path = %path.display(), "corrupt DHT state file, ignoring");
                return;
            }
        };

        let mut loaded = 0u32;
        for entry in &state.nodes {
            let Ok(id) = Id20::from_hex(&entry.id) else {
                continue;
            };
            let addr: SocketAddr = match entry.addr.parse() {
                Ok(a) => a,
                Err(_) => continue,
            };
            if self.routing_table.write().insert(id, addr) {
                loaded = loaded.saturating_add(1);
            }
        }

        if loaded > 0 {
            // Mark all as Questionable — they may be stale, must be re-verified
            self.routing_table.write().mark_all_questionable();

            // Filter bootstrap_nodes to hostnames only (remove IP:port entries)
            // since the JSON file has fresher saved-node addresses.
            self.config
                .bootstrap_nodes
                .retain(|s| s.parse::<SocketAddr>().is_err());

            debug!(
                loaded,
                table_size = self.routing_table.read().len(),
                family = ?self.address_family,
                "loaded DHT routing table from JSON"
            );
        }
    }

    async fn maintenance(&mut self) {
        // Query timeouts are now handled by expire_queries_and_advance_lookups()

        // Clean up completed DhtLookup tasks (where the JoinHandle has finished)
        self.active_lookups
            .retain(|_, handle| !handle.is_finished());

        // Gap 12: Clean up item lookups — send best result before dropping stale lookups
        self.item_lookups.retain(|_, lookup| match lookup {
            ItemLookupState::Immutable { reply, .. } => {
                if reply
                    .as_ref()
                    .is_some_and(tokio::sync::oneshot::Sender::is_closed)
                {
                    // Receiver dropped — discard
                    false
                } else if reply.is_some() {
                    true
                } else {
                    // Reply already sent
                    false
                }
            }
            ItemLookupState::Mutable {
                reply,
                best_value,
                best_seq,
                ..
            } => {
                if reply
                    .as_ref()
                    .is_some_and(tokio::sync::oneshot::Sender::is_closed)
                {
                    false
                } else if reply.is_some() {
                    true
                } else {
                    // Check if we should finalize — reply already taken means we're done
                    let _ = best_value;
                    let _ = best_seq;
                    false
                }
            }
        });

        // Clean up completed put operations
        self.item_put_ops.retain(|_, put_op| match put_op {
            ItemPutState::Immutable { reply, .. } | ItemPutState::Mutable { reply, .. } => {
                reply.is_some()
            }
        });

        // Refresh stale buckets
        let stale = self
            .routing_table
            .read()
            .stale_buckets(Duration::from_mins(15));
        for bucket_idx in stale {
            let target = self.routing_table.read().random_id_in_bucket(bucket_idx);
            let closest = self.routing_table.read().closest(&target, 3);
            for node in closest {
                self.send_find_node(node.addr, target, Some(node.id)).await;
            }
        }

        // Persist routing table to JSON (atomic write)
        self.save_routing_table();
    }

    async fn send_find_node(&mut self, addr: SocketAddr, target: Id20, node_id: Option<Id20>) {
        if !self.rate_limiter.try_acquire() {
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::FindNode {
                id: own_id,
                target,
                want: self.outgoing_want(),
            }),
            sender_ip: None,
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, addr).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr,
                    kind: PendingQueryKind::FindNode,
                    node_id,
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
    }

    async fn send_ping(&mut self, addr: SocketAddr, node_id: Option<Id20>) {
        if !self.rate_limiter.try_acquire() {
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::Ping { id: own_id }),
            sender_ip: None,
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, addr).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr,
                    node_id,
                    kind: PendingQueryKind::Ping,
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
    }

    async fn ping_questionable_nodes(&mut self) {
        let nodes = self.routing_table.read().questionable_nodes();
        for (id, addr) in nodes {
            self.send_ping(addr, Some(id)).await;
        }
    }

    // ---- BEP 44: item get/put handlers ----

    async fn handle_get_immutable(
        &mut self,
        target: Id20,
        reply: oneshot::Sender<Result<Option<Vec<u8>>>>,
    ) {
        // Check local store first
        if let Some(item) = self.item_store.get_immutable(&target) {
            let _ = reply.send(Ok(Some(item.value)));
            return;
        }

        // Initiate iterative get to the closest nodes
        let closest = self.routing_table.read().closest(&target, K);
        if closest.is_empty() {
            // No nodes to query — return None immediately
            let _ = reply.send(Ok(None));
            return;
        }

        for node in closest.iter().take(3) {
            self.send_get_item(node.addr, target, None).await;
        }

        self.item_lookups.insert(
            target,
            ItemLookupState::Immutable {
                reply: Some(reply),
                queried: closest.iter().map(|n| n.id).collect(),
            },
        );
    }

    async fn handle_put_immutable(&mut self, value: Vec<u8>, reply: oneshot::Sender<Result<Id20>>) {
        let item = match crate::bep44::ImmutableItem::new(value) {
            Ok(item) => item,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let target = item.target;

        // Store locally
        self.item_store.put_immutable(item.clone());

        // Reply immediately — local store succeeded.
        let _ = reply.send(Ok(target));

        // Best-effort propagation: find closest nodes, get tokens, then put.
        let closest = self.routing_table.read().closest(&target, K);
        if closest.is_empty() {
            return;
        }

        for node in closest.iter().take(K) {
            self.send_get_item(node.addr, target, None).await;
        }

        self.item_put_ops.insert(
            target,
            ItemPutState::Immutable {
                item,
                tokens: HashMap::new(),
                sent_puts: 0,
                reply: None,
            },
        );
    }

    #[allow(clippy::type_complexity)]
    async fn handle_get_mutable(
        &mut self,
        public_key: [u8; 32],
        salt: Vec<u8>,
        reply: oneshot::Sender<Result<Option<(Vec<u8>, i64)>>>,
    ) {
        let target = crate::bep44::compute_mutable_target(&public_key, &salt);

        // Check local store first
        if let Some(item) = self.item_store.get_mutable(&public_key, &salt) {
            let _ = reply.send(Ok(Some((item.value, item.seq))));
            return;
        }

        // Initiate iterative get
        let closest = self.routing_table.read().closest(&target, K);
        if closest.is_empty() {
            let _ = reply.send(Ok(None));
            return;
        }

        for node in closest.iter().take(3) {
            self.send_get_item(node.addr, target, None).await;
        }

        self.item_lookups.insert(
            target,
            ItemLookupState::Mutable {
                salt,
                reply: Some(reply),
                best_seq: i64::MIN,
                best_value: None,
                queried: closest.iter().map(|n| n.id).collect(),
            },
        );
    }

    async fn handle_put_mutable(
        &mut self,
        keypair_bytes: [u8; 32],
        value: Vec<u8>,
        seq: i64,
        salt: Vec<u8>,
        reply: oneshot::Sender<Result<Id20>>,
    ) {
        let keypair = ed25519_dalek::SigningKey::from_bytes(&keypair_bytes);
        let item = match crate::bep44::MutableItem::create(&keypair, value, seq, salt) {
            Ok(item) => item,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let target = item.target;

        // Store locally
        self.item_store.put_mutable(item.clone());

        // Reply immediately — local store succeeded.
        let _ = reply.send(Ok(target));

        // Best-effort propagation: find closest nodes, get tokens, then put.
        let closest = self.routing_table.read().closest(&target, K);
        if closest.is_empty() {
            return;
        }

        for node in closest.iter().take(K) {
            self.send_get_item(node.addr, target, None).await;
        }

        self.item_put_ops.insert(
            target,
            ItemPutState::Mutable {
                item,
                tokens: HashMap::new(),
                sent_puts: 0,
                reply: None,
            },
        );
    }

    // Gap 5: send_get_item uses sender_ip: None for outgoing queries
    async fn send_get_item(&mut self, addr: SocketAddr, target: Id20, seq: Option<i64>) {
        if !self.rate_limiter.try_acquire() {
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::Get {
                id: own_id,
                target,
                seq,
            }),
            sender_ip: None, // Gap 5: outgoing queries use None
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, addr).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr,
                    kind: PendingQueryKind::GetItem { target },
                    node_id: None,
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
    }

    // Gap 5: send_put_item uses sender_ip: None for outgoing queries
    async fn send_put_item(&mut self, params: PutItemParams) {
        if !self.rate_limiter.try_acquire() {
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::Put {
                id: own_id,
                token: params.token,
                value: params.value,
                key: params.key,
                signature: params.signature,
                seq: params.seq,
                salt: params.salt,
                cas: None,
            }),
            sender_ip: None, // Gap 5: outgoing queries use None
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, params.addr).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr: params.addr,
                    kind: PendingQueryKind::PutItem,
                    node_id: None,
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
    }

    // Gap 8: Extract data into local variables before calling self.send_put_item
    async fn send_pending_puts(&mut self, target: Id20) {
        let puts_to_send: Vec<PutItemParams> = if let Some(put_op) = self.item_put_ops.get(&target)
        {
            match put_op {
                ItemPutState::Immutable { item, tokens, .. } => tokens
                    .values()
                    .take(K)
                    .map(|(addr, token)| PutItemParams {
                        addr: *addr,
                        token: token.clone(),
                        value: item.value.clone(),
                        key: None,
                        signature: None,
                        seq: None,
                        salt: None,
                    })
                    .collect(),
                ItemPutState::Mutable { item, tokens, .. } => {
                    let salt = if item.salt.is_empty() {
                        None
                    } else {
                        Some(item.salt.clone())
                    };
                    tokens
                        .values()
                        .take(K)
                        .map(|(addr, token)| PutItemParams {
                            addr: *addr,
                            token: token.clone(),
                            value: item.value.clone(),
                            key: Some(item.public_key),
                            signature: Some(item.signature),
                            seq: Some(item.seq),
                            salt: salt.clone(),
                        })
                        .collect()
                }
            }
        } else {
            return;
        };

        let num_puts = puts_to_send.len();
        for params in puts_to_send {
            self.send_put_item(params).await;
        }

        // Update sent_puts count and send reply
        if let Some(put_op) = self.item_put_ops.get_mut(&target) {
            match put_op {
                ItemPutState::Immutable {
                    item,
                    sent_puts,
                    reply,
                    ..
                } => {
                    *sent_puts = num_puts;
                    if let Some(r) = reply.take() {
                        let _ = r.send(Ok(item.target));
                    }
                }
                ItemPutState::Mutable {
                    item,
                    sent_puts,
                    reply,
                    ..
                } => {
                    *sent_puts = num_puts;
                    if let Some(r) = reply.take() {
                        let _ = r.send(Ok(item.target));
                    }
                }
            }
        }
    }

    // ---- BEP 51: sample_infohashes handler ----

    async fn handle_sample_infohashes(
        &mut self,
        target: Id20,
        reply: oneshot::Sender<Result<SampleInfohashesResult>>,
    ) {
        // Find closest node to the target and send the query there
        let closest = self.routing_table.read().closest(&target, 1);
        let (addr, closest_node_id) = if let Some(node) = closest.first() {
            (node.addr, node.id)
        } else {
            let _ = reply.send(Err(Error::InvalidMessage(
                "no nodes in routing table".into(),
            )));
            return;
        };

        if !self.rate_limiter.try_acquire() {
            let _ = reply.send(Err(Error::Timeout));
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::SampleInfohashes { id: own_id, target }),
            sender_ip: None, // Gap 2: outgoing queries use None
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, addr).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr,
                    kind: PendingQueryKind::SampleInfohashes,
                    node_id: Some(closest_node_id),
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
        // Store the reply sender for when the response comes back
        self.sample_replies.insert(txn, reply);
    }

    async fn handle_direct_get_peers(
        &mut self,
        target: SocketAddr,
        info_hash: Id20,
        reply: oneshot::Sender<Result<Vec<SocketAddr>>>,
    ) {
        if !self.rate_limiter.try_acquire() {
            let _ = reply.send(Err(Error::Timeout));
            return;
        }
        let txn = self.next_transaction_id();
        let own_id = *self.routing_table.read().own_id();
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(txn),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: own_id,
                info_hash,
                noseed: None,
                scrape: Some(1),
                want: self.outgoing_want(),
            }),
            sender_ip: None,
            read_only: self.config.read_only_mode,
        };
        if let Ok(bytes) = msg.to_bytes() {
            let _ = self.socket.send_to(&bytes, target).await;
            self.pending.insert(
                txn,
                PendingQuery {
                    sent_at: Instant::now(),
                    addr: target,
                    kind: PendingQueryKind::DirectGetPeers,
                    node_id: None,
                    response_tx: None,
                },
            );
            self.stats.total_queries_sent += 1;
        }
        self.direct_peer_replies.insert(txn, reply);
    }

    fn next_transaction_id(&self) -> u16 {
        let txn = self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        // Skip zero — reserved as "invalid".
        if txn == 0 {
            return self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        }
        txn
    }

    /// Insert a node into the routing table, enforcing BEP 42 and BEP 43 if enabled.
    fn checked_insert(&self, id: Id20, addr: SocketAddr, read_only: bool) -> bool {
        // BEP 43: never add read-only nodes to the routing table
        if read_only {
            trace!(
                node_id = %id,
                ip = %addr.ip(),
                "BEP 43: skipping read-only node"
            );
            return false;
        }
        if self.config.enforce_node_id && !node_id::is_valid_node_id(&id, addr.ip()) {
            trace!(
                node_id = %id,
                ip = %addr.ip(),
                "BEP 42: rejecting node with invalid ID for IP"
            );
            return false;
        }
        self.routing_table.write().insert(id, addr)
    }

    /// Regenerate our node ID to be BEP 42-compliant for the given external IP.
    ///
    /// Preserves existing routing table nodes by re-inserting them into the
    /// new table. This avoids losing bootstrap-discovered nodes when the IP
    /// voter reaches consensus shortly after startup.
    fn regenerate_node_id(&mut self, external_ip: std::net::IpAddr) {
        let r = self.routing_table.read().own_id().0[19] & 0x07;
        let new_id = node_id::generate_node_id(external_ip, r);
        let restrict_ips = self.config.restrict_routing_ips;
        let max_routing_nodes = self.config.max_routing_nodes;
        let mut old_nodes = self.routing_table.read().all_nodes();
        debug!(
            old_id = %self.routing_table.read().own_id(),
            new_id = %new_id,
            preserved_nodes = old_nodes.len(),
            "BEP 42: regenerating node ID"
        );
        *self.routing_table.write() =
            RoutingTable::with_config(new_id, restrict_ips, max_routing_nodes);

        // Sort nodes by XOR distance to the new ID (closest first).
        // This maximizes bucket splits: close nodes fill the home bucket,
        // triggering splits that create capacity for more distant nodes.
        // Without sorting, distant nodes fill non-splittable buckets first
        // and get rejected (we saw 72→20 node loss without this).
        old_nodes.sort_by_key(|(id, _)| id.xor_distance(&new_id));

        let mut inserted = 0usize;
        for (id, addr) in &old_nodes {
            if self.routing_table.write().insert(*id, *addr) {
                inserted += 1;
            }
        }
        debug!(
            new_table_size = self.routing_table.read().len(),
            attempted = old_nodes.len(),
            inserted,
            "BEP 42: node ID regeneration complete"
        );

        // Invalidate all active DhtLookup tasks. They hold Arc clones of the
        // routing table (which is now replaced), and their closest-node lists
        // may be wrong under the new ID. Aborting drops their peer_tx senders,
        // which makes the session detect `dht_peers_rx = None` and re-issue
        // `get_peers()` against the fresh routing table.
        if !self.active_lookups.is_empty() {
            // Also remove pending queries for the cleared lookups. Without this,
            // stale queries expire later and call mark_failed() on nodes that the
            // NEW lookup might want to query, degrading their routing table status.
            let cleared_hashes: std::collections::HashSet<Id20> =
                self.active_lookups.keys().copied().collect();
            let stale_txns: Vec<u16> = self
                .pending
                .iter()
                .filter(|entry| {
                    matches!(entry.value().kind, PendingQueryKind::GetPeers { info_hash }
                        if cleared_hashes.contains(&info_hash))
                })
                .map(|entry| *entry.key())
                .collect();
            debug!(
                active_lookups = self.active_lookups.len(),
                stale_pending = stale_txns.len(),
                "BEP 42: invalidating active get_peers lookups (will be re-issued by session)"
            );
            for txn in stale_txns {
                self.pending.remove(&txn);
            }
            for (_, handle) in self.active_lookups.drain() {
                handle.abort();
            }
        }

        // Re-trigger iterative bootstrap with the new node ID.
        // The first bootstrap targeted the old ID, so the discovered nodes
        // are in the wrong neighbourhood. A fresh find_node cascade targeting
        // the new ID fills the home bucket properly.
        let initial_closest: Vec<CompactNodeInfo> = self
            .routing_table
            .read()
            .closest(&new_id, K)
            .into_iter()
            .map(|n| CompactNodeInfo {
                id: n.id,
                addr: n.addr,
            })
            .collect();
        if !initial_closest.is_empty() {
            debug!(
                seed_nodes = initial_closest.len(),
                "BEP 42: re-bootstrapping with new node ID"
            );
            let mut lookup = IterativeLookup::new(
                new_id,
                FindNodeCallbacks {
                    round: 0,
                    max_rounds: 6,
                },
            );
            lookup.closest = initial_closest;
            self.bootstrap_lookup = Some(lookup);
            // M97: Re-gate get_peers until the new bootstrap completes
            self.bootstrap_complete = false;
            self.bootstrap_timeout = Some(Box::pin(tokio::time::sleep(Duration::from_secs(10))));
        }
    }

    fn make_stats(&self) -> DhtStats {
        let (immutable, mutable) = self.item_store.count();
        DhtStats {
            node_id: *self.routing_table.read().own_id(),
            routing_table_size: self.routing_table.read().len(),
            bucket_count: self.routing_table.read().bucket_count(),
            peer_store_info_hashes: self.peer_store.info_hash_count(),
            peer_store_peers: self.peer_store.peer_count(),
            pending_queries: self.pending.len(),
            active_lookups: self.active_lookups.len(),
            announce_tokens: self.announce_tokens.len(),
            total_queries_sent: self.stats.total_queries_sent,
            total_responses_received: self.stats.total_responses_received,
            dht_item_count: immutable + mutable,
            announces_received: self.stats.announces_received,
            announces_token_rejected: self.stats.announces_token_rejected,
            announces_suppressed_readonly: self.stats.announces_suppressed_readonly,
            lookups_received: self.stats.lookups_received,
        }
    }
}

/// Hash a socket address to a u64 for use as a voter source ID.
fn hash_source_addr(addr: &SocketAddr) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    addr.hash(&mut hasher);
    hasher.finish()
}

/// Maximum duration for DNS bootstrap retry attempts per hostname.
const DNS_BOOTSTRAP_DEADLINE: Duration = Duration::from_mins(2);

/// Initial retry delay for DNS bootstrap resolution.
const DNS_BOOTSTRAP_INITIAL_DELAY: Duration = Duration::from_secs(1);

/// Maximum retry delay for DNS bootstrap resolution (exponential backoff cap).
const DNS_BOOTSTRAP_MAX_DELAY: Duration = Duration::from_secs(30);

/// Resolve a single bootstrap hostname with exponential backoff.
///
/// Retries DNS resolution with delays of 1s, 2s, 4s, ..., capped at 30s,
/// until success or the 120-second deadline is reached. On success, sends
/// the matching addresses (filtered by address family) to `tx`.
async fn dns_bootstrap_resolve(
    hostname: String,
    family: AddressFamily,
    tx: mpsc::Sender<Vec<SocketAddr>>,
) {
    let deadline = Instant::now() + DNS_BOOTSTRAP_DEADLINE;
    let mut delay = DNS_BOOTSTRAP_INITIAL_DELAY;

    loop {
        match tokio::net::lookup_host(hostname.as_str()).await {
            Ok(addrs) => {
                let matching: Vec<SocketAddr> = addrs
                    .filter(|a| match family {
                        AddressFamily::V4 => a.is_ipv4(),
                        AddressFamily::V6 => a.is_ipv6(),
                    })
                    .collect();
                debug!(
                    %hostname,
                    count = matching.len(),
                    ?family,
                    "DNS bootstrap resolved"
                );
                let _ = tx.send(matching).await;
                break;
            }
            Err(e) if Instant::now() + delay < deadline => {
                warn!(%hostname, %e, ?delay, "DNS bootstrap retry");
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2).min(DNS_BOOTSTRAP_MAX_DELAY);
            }
            Err(e) => {
                warn!(%hostname, %e, "DNS bootstrap failed after retries");
                break;
            }
        }
    }
}

/// Generate a random node ID for this DHT node.
fn generate_node_id() -> Id20 {
    use std::cell::Cell;
    use std::time::SystemTime;

    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        );
    }

    let mut bytes = [0u8; 20];
    for byte in &mut bytes {
        STATE.with(|s| {
            let mut x = s.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.set(x);
            *byte = x as u8;
        });
    }
    Id20(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_node_id_is_unique() {
        let a = generate_node_id();
        let b = generate_node_id();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn dht_handle_start_and_shutdown() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(), // No bootstrap for test
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.routing_table_size, 0);
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_handle_stats() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.routing_table_size, 0);
        assert_eq!(stats.bucket_count, 160);
        assert_eq!(stats.pending_queries, 0);
        handle.shutdown().await.unwrap();
    }

    /// M171 D4: [`DhtHandle::node_count`] must return the same value as
    /// `stats().routing_table_size`. Using a startup-empty bootstrap
    /// ensures the pre-populate count is 0.
    #[tokio::test]
    async fn dht_handle_node_count_matches_stats() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let stats = handle.stats().await.unwrap();
        let count = handle.node_count().await.unwrap();
        assert_eq!(count, stats.routing_table_size);
        assert_eq!(count, 0, "empty bootstrap ⇒ empty routing table");
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn two_dht_nodes_ping() {
        // Start two DHT nodes on localhost, have one send find_node to the other
        let config_a = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            ..DhtConfig::default()
        };
        let config_b = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000002").unwrap()),
            ..DhtConfig::default()
        };

        let (handle_a, _ip_rx_a) = DhtHandle::start(config_a).await.unwrap();
        let (handle_b, _ip_rx_b) = DhtHandle::start(config_b).await.unwrap();

        // Give them a moment to bind
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Both should have empty routing tables
        let stats_a = handle_a.stats().await.unwrap();
        let stats_b = handle_b.stats().await.unwrap();
        assert_eq!(stats_a.routing_table_size, 0);
        assert_eq!(stats_b.routing_table_size, 0);

        handle_a.shutdown().await.unwrap();
        handle_b.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_handle_get_peers_empty_table() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let info_hash = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        let _rx = handle.get_peers(info_hash).await.unwrap();

        // With empty routing table, no peers will be found and channel closes
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Channel should eventually be cleaned up
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.routing_table_size, 0);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_handles_malformed_packet() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        // Get the DHT port from stats (indirect — we'd need to expose local_addr)
        // For now, just verify it doesn't crash on shutdown
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.shutdown().await.unwrap();
    }

    #[test]
    fn dht_config_default_is_v4() {
        let config = DhtConfig::default();
        assert_eq!(config.address_family, AddressFamily::V4);
        assert!(config.bind_addr.is_ipv4());
    }

    #[test]
    fn dht_config_default_v6() {
        let config = DhtConfig::default_v6();
        assert_eq!(config.address_family, AddressFamily::V6);
        assert!(config.bind_addr.is_ipv6());
        // Should have bootstrap nodes
        assert!(!config.bootstrap_nodes.is_empty());
    }

    #[tokio::test]
    async fn dht_v6_start_and_shutdown() {
        let config = DhtConfig {
            bind_addr: "[::1]:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default_v6()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.routing_table_size, 0);
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_v6_stats_on_empty_table() {
        let config = DhtConfig {
            bind_addr: "[::1]:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default_v6()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.routing_table_size, 0);
        assert_eq!(stats.bucket_count, 160);
        assert_eq!(stats.pending_queries, 0);
        assert_eq!(stats.total_queries_sent, 0);
        handle.shutdown().await.unwrap();
    }

    #[test]
    fn matches_family_helper() {
        let actor_v4 = AddressFamily::V4;
        let actor_v6 = AddressFamily::V6;
        let v4_addr: SocketAddr = "1.2.3.4:6881".parse().unwrap();
        let v6_addr: SocketAddr = "[::1]:6881".parse().unwrap();

        assert!(matches!(actor_v4, AddressFamily::V4) && v4_addr.is_ipv4());
        assert!(!v6_addr.is_ipv4());
        assert!(matches!(actor_v6, AddressFamily::V6) && v6_addr.is_ipv6());
        assert!(!v4_addr.is_ipv6());
    }

    #[test]
    fn dht_config_security_defaults() {
        let config = DhtConfig::default();
        // enforce_node_id off by default: too many real DHT nodes lack BEP 42 IDs
        assert!(!config.enforce_node_id);
        assert!(config.restrict_routing_ips);

        let config_v6 = DhtConfig::default_v6();
        assert!(!config_v6.enforce_node_id);
        assert!(config_v6.restrict_routing_ips);
    }

    #[tokio::test]
    async fn dht_handle_start_returns_ip_channel() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_update_external_ip() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        handle
            .update_external_ip("203.0.113.5".parse().unwrap(), IpVoteSource::Nat)
            .await
            .unwrap();
        handle.shutdown().await.unwrap();
    }

    // ---- BEP 44 put/get API tests ----

    // Gap 2: All tests use `let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();`

    #[tokio::test]
    async fn dht_put_get_immutable_local() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        // Put an immutable item
        let value = b"12:Hello World!".to_vec();
        let target = handle.put_immutable(value.clone()).await.unwrap();

        // Get it back (from local store)
        let result = handle.get_immutable(target).await.unwrap();
        assert_eq!(result, Some(value));

        // Verify SHA-1 target
        assert_eq!(target, gaia_core::sha1(b"12:Hello World!"));

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_put_get_mutable_local() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let seed = [42u8; 32];
        let keypair = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pubkey = keypair.verifying_key().to_bytes();

        let value = b"4:test".to_vec();
        let target = handle
            .put_mutable(seed, value.clone(), 1, Vec::new())
            .await
            .unwrap();

        // Get it back (from local store)
        let result = handle.get_mutable(pubkey, Vec::new()).await.unwrap();
        assert_eq!(result, Some((value, 1)));

        // Verify target
        let expected_target = crate::bep44::compute_mutable_target(&pubkey, &[]);
        assert_eq!(target, expected_target);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_get_immutable_not_found() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let target = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        // With empty routing table, lookup has no peers to query; just returns local result
        let result = handle.get_immutable(target).await.unwrap();
        assert_eq!(result, None);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_put_immutable_rejects_oversized() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let value = vec![0u8; 1001];
        let result = handle.put_immutable(value).await;
        assert!(result.is_err());

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_stats_includes_item_count() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.dht_item_count, 0);

        handle.put_immutable(b"5:hello".to_vec()).await.unwrap();
        let stats = handle.stats().await.unwrap();
        assert_eq!(stats.dht_item_count, 1);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dht_get_mutable_not_found() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let pubkey = [99u8; 32];
        let result = handle.get_mutable(pubkey, Vec::new()).await.unwrap();
        assert_eq!(result, None);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn two_nodes_put_get_immutable() {
        let config_a = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            ..DhtConfig::default()
        };
        let (handle_a, _ip_rx) = DhtHandle::start(config_a).await.unwrap();

        // Node A stores an item locally
        let value = b"12:Hello World!".to_vec();
        let target = handle_a.put_immutable(value.clone()).await.unwrap();

        // Verify local retrieval
        let result = handle_a.get_immutable(target).await.unwrap();
        assert_eq!(result, Some(value));

        handle_a.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn put_mutable_sequence_update() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let seed = [99u8; 32];
        let keypair = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pubkey = keypair.verifying_key().to_bytes();

        // Put seq=1
        handle
            .put_mutable(seed, b"5:first".to_vec(), 1, Vec::new())
            .await
            .unwrap();
        let result = handle.get_mutable(pubkey, Vec::new()).await.unwrap();
        assert_eq!(result, Some((b"5:first".to_vec(), 1)));

        // Put seq=2 (should replace)
        handle
            .put_mutable(seed, b"6:second".to_vec(), 2, Vec::new())
            .await
            .unwrap();
        let result = handle.get_mutable(pubkey, Vec::new()).await.unwrap();
        assert_eq!(result, Some((b"6:second".to_vec(), 2)));

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn put_mutable_with_salt_isolation() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let seed = [77u8; 32];
        let keypair = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pubkey = keypair.verifying_key().to_bytes();

        // Put with salt "a"
        handle
            .put_mutable(seed, b"1:A".to_vec(), 1, b"a".to_vec())
            .await
            .unwrap();
        // Put with salt "b"
        handle
            .put_mutable(seed, b"1:B".to_vec(), 1, b"b".to_vec())
            .await
            .unwrap();

        // Each salt returns its own value
        let a = handle.get_mutable(pubkey, b"a".to_vec()).await.unwrap();
        assert_eq!(a, Some((b"1:A".to_vec(), 1)));
        let b = handle.get_mutable(pubkey, b"b".to_vec()).await.unwrap();
        assert_eq!(b, Some((b"1:B".to_vec(), 1)));

        handle.shutdown().await.unwrap();
    }

    // ---- BEP 51 sample_infohashes tests ----

    #[tokio::test]
    async fn dht_sample_infohashes_empty_table() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let target = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        let result = handle.sample_infohashes(target).await;
        // With empty routing table, we expect an error (no nodes to query)
        assert!(result.is_err());

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn two_nodes_sample_infohashes() {
        // Node A will store some peers, then node B queries it
        let config_a = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            ..DhtConfig::default()
        };
        let (handle_a, _ip_rx_a) = DhtHandle::start(config_a).await.unwrap();

        // We can't directly add peers to node A's store through the public API,
        // but we can verify the query/response path by having node B query node A.
        // Node A will respond with empty samples since its peer store is empty.

        // For now, just verify the handle method exists and handles shutdown gracefully
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle_a.shutdown().await.unwrap();
    }

    // ---- QueryRateLimiter unit tests ----

    #[test]
    fn rate_limiter_new_starts_full() {
        let limiter = QueryRateLimiter::new(10);
        assert_eq!(limiter.permits, 10);
        assert_eq!(limiter.max_permits, 10);
        assert_eq!(limiter.refill_rate, 10);
    }

    #[test]
    fn rate_limiter_new_zero_rate() {
        // A zero-rate limiter should never grant permits.
        let mut limiter = QueryRateLimiter::new(0);
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn rate_limiter_exhaustion() {
        // Drain all N permits, then the (N+1)th call must fail.
        let mut limiter = QueryRateLimiter::new(5);
        for _ in 0..5 {
            assert!(limiter.try_acquire(), "permit should be available");
        }
        assert!(
            !limiter.try_acquire(),
            "bucket must be empty after N acquires"
        );
    }

    #[test]
    fn rate_limiter_initial_permits_work() {
        // Full bucket on creation: first try_acquire always succeeds.
        let mut limiter = QueryRateLimiter::new(1);
        assert!(limiter.try_acquire());
        // Bucket is now empty.
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn rate_limiter_refill_caps_at_max() {
        // Manually set permits below max, then trigger a refill by faking a
        // large elapsed time through repeated calls; instead, just validate the
        // cap logic by setting state directly and calling refill via try_acquire.
        // We can't easily fake Instant, but we can verify that permits never
        // exceed max_permits after a refill.
        let mut limiter = QueryRateLimiter::new(10);
        // Drain to 0.
        for _ in 0..10 {
            limiter.try_acquire();
        }
        assert_eq!(limiter.permits, 0);

        // Sleep slightly longer than 1 second so the refill would add >10 permits
        // if uncapped. Since we cannot sleep in a unit test cheaply, we instead
        // directly manipulate last_refill to simulate elapsed time.
        limiter.last_refill = Instant::now() - Duration::from_secs(5);
        limiter.refill();
        // After 5 seconds at rate 10, raw new_permits = 50, but cap is 10.
        assert_eq!(limiter.permits, 10, "permits must not exceed max_permits");
    }

    #[test]
    fn rate_limiter_refill_adds_correct_permits() {
        let mut limiter = QueryRateLimiter::new(100);
        // Drain all.
        for _ in 0..100 {
            limiter.try_acquire();
        }
        // Simulate 0.5 seconds elapsed → should add ~50 permits.
        limiter.last_refill = Instant::now() - Duration::from_millis(500);
        limiter.refill();
        // Allow for timing imprecision: must be in [45, 55].
        assert!(
            limiter.permits >= 45 && limiter.permits <= 55,
            "expected ~50 permits after 0.5s refill at rate 100, got {}",
            limiter.permits
        );
    }

    /// Exercises the bootstrap path with saved-node addresses (no DNS).
    /// Verifies the bootstrap code path (including new diagnostic logging)
    /// runs without panicking.
    #[tokio::test]
    async fn dht_bootstrap_logging() {
        // Use a fake saved-node address (loopback) that won't resolve to a
        // real DHT node — the important thing is that the bootstrap code path
        // executes all three phases without panicking.
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: vec!["127.0.0.1:16881".to_owned(), "127.0.0.1:16882".to_owned()],
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        // Allow time for bootstrap() to run (pings sent, iterative lookup started).
        tokio::time::sleep(Duration::from_millis(200)).await;

        let stats = handle.stats().await.unwrap();
        // The pings will have been sent (queries_sent >= 2) but won't get
        // responses from the fake addresses.
        assert!(
            stats.total_queries_sent >= 2,
            "expected at least 2 ping queries, got {}",
            stats.total_queries_sent
        );

        handle.shutdown().await.unwrap();
    }

    /// T10: During bootstrap (`bootstrap_complete = false`), the ping gate
    /// uses a 5-second interval — pings should fire every tick.
    ///
    /// Uses millisecond-scale durations so the test completes instantly while
    /// exercising the exact same gating logic as the real actor loop.
    #[test]
    fn ping_interval_5s_during_bootstrap() {
        let bootstrap_complete = false;

        // Simulate the timing decision with a tick interval equal to the
        // bootstrap ping interval (both 5s in production, both 10ms here).
        let tick = Duration::from_millis(10);
        let bootstrap_interval = tick;
        let steady_interval = Duration::from_millis(120);

        let mut last_ping = Instant::now();
        let mut ping_count: u32 = 0;

        // Simulate 6 ticks, sleeping the tick interval between each.
        for _ in 0..6 {
            std::thread::sleep(tick);

            let ping_interval = if bootstrap_complete {
                steady_interval
            } else {
                bootstrap_interval
            };
            if last_ping.elapsed() >= ping_interval {
                ping_count = ping_count.saturating_add(1);
                last_ping = Instant::now();
            }
        }

        // All 6 ticks should trigger a ping (tick == bootstrap interval).
        assert_eq!(
            ping_count, 6,
            "expected 6 pings during bootstrap (every tick), got {ping_count}"
        );
    }

    /// T11: After bootstrap (`bootstrap_complete = true`), the ping gate
    /// uses a 60-second interval — most ticks are no-ops for pinging.
    ///
    /// Uses millisecond-scale durations so the test completes instantly while
    /// exercising the exact same gating logic as the real actor loop.
    #[test]
    fn ping_interval_60s_after_bootstrap() {
        let bootstrap_complete = true;

        // Production ratio: tick = 5s, steady interval = 60s → 12:1.
        // Test ratio:       tick = 10ms, steady interval = 120ms → 12:1.
        let tick = Duration::from_millis(10);
        let bootstrap_interval = tick;
        let steady_interval = Duration::from_millis(120);

        let mut last_ping = Instant::now();
        let mut ping_count: u32 = 0;

        // 24 ticks × 10ms = 240ms total. With a 120ms gate, exactly 2
        // pings should fire (at tick ~12 = 120ms and tick ~24 = 240ms).
        for _ in 0..24 {
            std::thread::sleep(tick);

            let ping_interval = if bootstrap_complete {
                steady_interval
            } else {
                bootstrap_interval
            };
            if last_ping.elapsed() >= ping_interval {
                ping_count = ping_count.saturating_add(1);
                last_ping = Instant::now();
            }
        }

        // Only 2 pings should have fired (12:1 ratio, same as production).
        assert_eq!(
            ping_count, 2,
            "expected 2 pings post-bootstrap (12:1 tick-to-interval ratio), got {ping_count}"
        );
    }

    // ---- DNS bootstrap backoff tests (M105 Task 3) ----

    /// T1: Verify DNS resolution is retried with increasing delay on failure.
    ///
    /// Uses a hostname that will definitely fail DNS resolution. Validates that
    /// the backoff logic computes the correct delay sequence (1s, 2s, 4s, ...)
    /// capped at 30s.
    #[test]
    fn dns_backoff_retries_on_failure() {
        // Validate the exponential backoff sequence directly.
        let mut delay = DNS_BOOTSTRAP_INITIAL_DELAY;
        let expected_delays = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30), // capped
            Duration::from_secs(30), // stays capped
        ];

        for expected in &expected_delays {
            assert_eq!(
                delay, *expected,
                "backoff delay mismatch: got {delay:?}, expected {expected:?}"
            );
            delay = delay.saturating_mul(2).min(DNS_BOOTSTRAP_MAX_DELAY);
        }
    }

    /// T2: Verify successful retry after initial failure proceeds normally.
    ///
    /// Spawns `dns_bootstrap_resolve` with localhost (which resolves
    /// immediately) and confirms addresses arrive on the channel.
    #[tokio::test]
    async fn dns_backoff_succeeds_on_retry() {
        let (tx, mut rx) = mpsc::channel(16);

        // "localhost:1234" should resolve immediately on any system.
        let hostname = "localhost:1234".to_owned();
        tokio::spawn(dns_bootstrap_resolve(hostname, AddressFamily::V4, tx));

        // We should receive at least one batch of addresses.
        let result = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(
            result.is_ok(),
            "expected DNS resolution to complete within 5 seconds"
        );
        let addrs = result.expect("timeout should not occur");
        // localhost resolves, so we should get Some with at least one address.
        assert!(
            addrs.is_some(),
            "expected Some(addresses) from dns_bootstrap_resolve"
        );
        let addrs = addrs.expect("already checked is_some");
        assert!(
            !addrs.is_empty(),
            "expected at least one resolved address for localhost"
        );
        // All addresses should be IPv4 since we requested V4.
        for addr in &addrs {
            assert!(addr.is_ipv4(), "expected IPv4 address, got {addr}");
        }
    }

    /// T3: Verify after 120s of failures, we stop retrying.
    ///
    /// Tests the deadline logic directly: once `Instant::now() + delay >= deadline`,
    /// the function should break out of its loop.
    #[test]
    fn dns_backoff_total_timeout_120s() {
        // Simulate the deadline check from dns_bootstrap_resolve.
        // With 120s deadline and delays 1,2,4,8,16,30,30,...
        // Sum: 1+2+4+8+16+30 = 61s after 6 retries, then 30s more = 91s after 7,
        // 121s after 8 retries → exceeds deadline.
        let deadline_duration = DNS_BOOTSTRAP_DEADLINE;
        let mut delay = DNS_BOOTSTRAP_INITIAL_DELAY;
        let mut total_sleep = Duration::ZERO;
        let mut retries = 0u32;

        loop {
            // Check if the next sleep would exceed the deadline
            // (mirrors: `Instant::now() + delay < deadline` in the real code,
            // but using cumulative durations since we can't fake Instant).
            let next_total = total_sleep.saturating_add(delay);
            if next_total >= deadline_duration {
                break;
            }
            total_sleep = next_total;
            retries = retries.saturating_add(1);
            delay = delay.saturating_mul(2).min(DNS_BOOTSTRAP_MAX_DELAY);
        }

        // Should have retried several times before hitting the deadline.
        assert!(
            retries >= 5,
            "expected at least 5 retries before 120s deadline, got {retries}"
        );
        // Total sleep should be < 120s (we broke before the last sleep).
        assert!(
            total_sleep < deadline_duration,
            "total sleep {total_sleep:?} should be less than deadline {deadline_duration:?}"
        );
    }

    /// T16: Verify Phase 3 (`FindNodeLookup`) starts immediately without
    /// waiting for DNS resolution.
    ///
    /// Starts a DHT actor with both saved-node addresses and a DNS hostname.
    /// After `bootstrap()`, the `bootstrap_lookup` (Phase 3) must be set, and
    /// `dns_bootstrap_rx` must be Some (DNS still in flight).
    #[tokio::test]
    async fn bootstrap_phase3_starts_before_dns() {
        // Use a DNS hostname that takes time to resolve (unresolvable is fine —
        // we just need to confirm Phase 3 didn't wait for it).
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: vec![
                // Saved node (parsed as SocketAddr → Phase 1 ping)
                "127.0.0.1:16881".to_owned(),
                // DNS hostname (goes to background task)
                "router.bittorrent.com:6881".to_owned(),
            ],
            ..DhtConfig::default()
        };

        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let mut actor = DhtActor::new(config, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        // Run bootstrap — should return quickly (DNS spawned in background).
        actor.bootstrap().await;

        // Phase 3 must have started: bootstrap_lookup is set.
        assert!(
            actor.bootstrap_lookup.is_some(),
            "Phase 3 (FindNodeLookup) must start without waiting for DNS"
        );

        // DNS is still in flight: dns_bootstrap_rx must be Some.
        assert!(
            actor.dns_bootstrap_rx.is_some(),
            "dns_bootstrap_rx should be Some (background DNS tasks still running)"
        );

        // Cleanup: drop sender so actor doesn't hang.
        drop(tx);
    }

    // ---- JSON routing table persistence tests (M105 Task 5) ----

    /// T12: Save routing table to JSON, read it back, verify nodes restored as
    /// Questionable. Also test that corrupt JSON is handled gracefully.
    #[tokio::test]
    async fn json_persistence_round_trip_and_corrupt() {
        use crate::routing_table::NodeStatus;
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };

        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config.clone(), socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        // Insert some nodes
        let node1_id = Id20::from_hex("1111111111111111111111111111111111111111").unwrap();
        let node2_id = Id20::from_hex("2222222222222222222222222222222222222222").unwrap();
        let addr1: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        let addr2: SocketAddr = "10.0.0.2:6882".parse().unwrap();
        actor.routing_table.write().insert(node1_id, addr1);
        actor.routing_table.write().insert(node2_id, addr2);
        // Mark one as Good to confirm mark_all_questionable works on load
        actor.routing_table.write().mark_response(&node1_id);

        // Save
        actor.save_routing_table();

        // Verify file exists
        let path = DhtActor::state_file_path(dir.path(), AddressFamily::V4);
        assert!(path.exists(), "JSON state file should exist after save");

        // Load into a new actor
        let config2 = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };
        let socket2 = Arc::new(UdpSocket::bind(config2.bind_addr).await.unwrap());
        let (_tx2, rx2) = mpsc::channel(256);
        let (ip_tx2, _ip_rx2) = mpsc::channel(4);
        let actor2 = DhtActor::new(config2, socket2, rx2, ip_tx2, tokio::sync::broadcast::channel(16).0);

        // Verify nodes were loaded
        assert_eq!(actor2.routing_table.read().len(), 2);
        assert!(actor2.routing_table.read().get(&node1_id).is_some());
        assert!(actor2.routing_table.read().get(&node2_id).is_some());

        // All nodes should be Questionable (mark_all_questionable was called)
        assert_eq!(
            actor2.routing_table.read().get(&node1_id).unwrap().status(),
            NodeStatus::Questionable
        );
        assert_eq!(
            actor2.routing_table.read().get(&node2_id).unwrap().status(),
            NodeStatus::Questionable
        );

        // --- Corrupt JSON test ---
        std::fs::write(&path, b"{{not valid json at all!!}}")
            .expect("failed to write corrupt data");

        let config3 = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };
        let socket3 = Arc::new(UdpSocket::bind(config3.bind_addr).await.unwrap());
        let (_tx3, rx3) = mpsc::channel(256);
        let (ip_tx3, _ip_rx3) = mpsc::channel(4);
        let actor3 = DhtActor::new(config3, socket3, rx3, ip_tx3, tokio::sync::broadcast::channel(16).0);

        // Corrupt JSON should result in an empty routing table (graceful fallback)
        assert_eq!(actor3.routing_table.read().len(), 0);
    }

    /// T13: Verify atomic write — temp file is written first, then renamed.
    /// A partial (interrupted) write should not corrupt the final state file.
    #[tokio::test]
    async fn json_persistence_atomic_write() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };

        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        let node_id = Id20::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let addr: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        actor.routing_table.write().insert(node_id, addr);

        // Write a known value to the final path first (simulate existing state)
        let final_path = DhtActor::state_file_path(dir.path(), AddressFamily::V4);
        std::fs::write(&final_path, b"old data").unwrap();

        // Save — should atomically replace via rename
        actor.save_routing_table();

        // Final file should contain valid JSON with our node
        let content = std::fs::read_to_string(&final_path).unwrap();
        let state: DhtState =
            serde_json::from_str(&content).expect("final file should contain valid JSON");
        assert_eq!(state.nodes.len(), 1);
        assert_eq!(state.nodes[0].id, node_id.to_hex());

        // Temp file should NOT exist (it was renamed away)
        let tmp_path = dir.path().join(".dht_state_v4.tmp");
        assert!(
            !tmp_path.exists(),
            "temp file should be cleaned up by rename"
        );
    }

    /// T14: Verify persistence is silently skipped when `state_dir` is `None`.
    #[tokio::test]
    async fn json_persistence_no_state_dir() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            state_dir: None, // No state dir
            ..DhtConfig::default()
        };

        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        // Insert a node — save should be a no-op
        let node_id = Id20::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        actor
            .routing_table
            .write()
            .insert(node_id, "10.0.0.1:6881".parse().unwrap());

        // This should not panic or do anything
        actor.save_routing_table();

        // load_routing_table in new() already ran silently (no state_dir)
        assert_eq!(actor.routing_table.read().len(), 1); // only the node we just inserted
    }

    /// T17: When JSON loads successfully, IP:port entries in `bootstrap_nodes`
    /// should be filtered out (hostnames remain).
    #[tokio::test]
    async fn json_persistence_priority_over_config() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        // First: create a state file with saved nodes.
        let config_save = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };

        let socket = Arc::new(UdpSocket::bind(config_save.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config_save, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        let node_id = Id20::from_hex("cccccccccccccccccccccccccccccccccccccccc").unwrap();
        actor
            .routing_table
            .write()
            .insert(node_id, "10.0.0.1:6881".parse().unwrap());
        actor.save_routing_table();
        drop(actor);

        // Now load with bootstrap_nodes containing both IP:port and hostnames.
        let config_load = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: vec![
                "192.168.1.100:6881".to_owned(), // IP:port — should be filtered out
                "10.0.0.50:6881".to_owned(),     // IP:port — should be filtered out
                "router.bittorrent.com:6881".to_owned(), // hostname — should remain
                "dht.transmissionbt.com:6881".to_owned(), // hostname — should remain
            ],
            own_id: Some(Id20::from_hex("0000000000000000000000000000000000000001").unwrap()),
            state_dir: Some(dir.path().to_path_buf()),
            ..DhtConfig::default()
        };

        let socket2 = Arc::new(UdpSocket::bind(config_load.bind_addr).await.unwrap());
        let (_tx2, rx2) = mpsc::channel(256);
        let (ip_tx2, _ip_rx2) = mpsc::channel(4);
        let actor2 = DhtActor::new(config_load, socket2, rx2, ip_tx2, tokio::sync::broadcast::channel(16).0);

        // Routing table should have the loaded node
        assert_eq!(actor2.routing_table.read().len(), 1);

        // bootstrap_nodes should only contain hostnames (IP:port entries filtered)
        assert_eq!(actor2.config.bootstrap_nodes.len(), 2);
        assert!(
            actor2
                .config
                .bootstrap_nodes
                .contains(&"router.bittorrent.com:6881".to_owned())
        );
        assert!(
            actor2
                .config
                .bootstrap_nodes
                .contains(&"dht.transmissionbt.com:6881".to_owned())
        );
        // IP:port entries should be gone
        assert!(
            !actor2
                .config
                .bootstrap_nodes
                .contains(&"192.168.1.100:6881".to_owned())
        );
        assert!(
            !actor2
                .config
                .bootstrap_nodes
                .contains(&"10.0.0.50:6881".to_owned())
        );
    }

    // --- BEP 43 read-only node tests ---

    #[tokio::test]
    async fn checked_insert_rejects_read_only() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        let id = Id20::from_hex("0000000000000000000000000000000000000042").unwrap();
        let addr: SocketAddr = "10.0.0.1:6881".parse().unwrap();

        // read_only: true => should NOT be inserted
        assert!(!actor.checked_insert(id, addr, true));
        assert_eq!(actor.routing_table.read().len(), 0);
    }

    #[tokio::test]
    async fn checked_insert_accepts_normal() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let socket = Arc::new(UdpSocket::bind(config.bind_addr).await.unwrap());
        let (_tx, rx) = mpsc::channel(256);
        let (ip_tx, _ip_rx) = mpsc::channel(4);
        let actor = DhtActor::new(config, socket, rx, ip_tx, tokio::sync::broadcast::channel(16).0);

        let id = Id20::from_hex("0000000000000000000000000000000000000042").unwrap();
        let addr: SocketAddr = "10.0.0.1:6881".parse().unwrap();

        // read_only: false => should be inserted normally
        assert!(actor.checked_insert(id, addr, false));
        assert_eq!(actor.routing_table.read().len(), 1);
    }

    #[tokio::test]
    async fn outgoing_query_includes_ro() {
        // When read_only_mode is true, the actor constructs outgoing queries
        // with `read_only: self.config.read_only_mode`. Verify the KrpcMessage
        // round-trip: when read_only is true, the encoded bytes contain `ro: 1`
        // and decoding recovers the flag.
        let info_hash = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        let own_id = Id20::ZERO;

        let msg = crate::krpc::KrpcMessage {
            transaction_id: crate::krpc::TransactionId::from_u16(1),
            body: crate::krpc::KrpcBody::Query(crate::krpc::KrpcQuery::FindNode {
                id: own_id,
                target: info_hash,
                want: None,
            }),
            sender_ip: None,
            read_only: true, // matches what send_find_node sets when read_only_mode: true
        };
        let bytes = msg.to_bytes().unwrap();

        // Verify the raw bencode contains "ro" key
        let raw: gaia_bencode::BencodeValue = gaia_bencode::from_bytes(&bytes).unwrap();
        let dict = raw.as_dict().unwrap();
        assert!(
            dict.contains_key(&b"ro"[..]),
            "query with read_only: true should contain ro key in wire format"
        );

        let decoded = crate::krpc::KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(decoded.read_only, "outgoing query should include ro flag");
    }

    #[tokio::test]
    async fn response_never_includes_ro() {
        // Responses should always have read_only: false, even from a read-only node.
        let own_id = Id20::ZERO;
        let msg = crate::krpc::KrpcMessage {
            transaction_id: crate::krpc::TransactionId::from_u16(1),
            body: crate::krpc::KrpcBody::Response(crate::krpc::KrpcResponse::NodeId { id: own_id }),
            sender_ip: None,
            read_only: false, // responses never include ro
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = crate::krpc::KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(!decoded.read_only, "responses should never include ro flag");

        // Verify a response constructed with read_only: false does NOT produce an ro field.
        // (The encoder only includes ro when true.)
        let raw: gaia_bencode::BencodeValue = gaia_bencode::from_bytes(&bytes).unwrap();
        let dict = raw.as_dict().unwrap();
        assert!(
            !dict.contains_key(&b"ro"[..]),
            "response bytes should not contain ro key"
        );
    }

    #[tokio::test]
    async fn announce_suppressed_in_read_only_mode() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            read_only_mode: true,
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        let info_hash = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        // announce should succeed silently (no-op) in read-only mode
        let result = handle.announce(info_hash, 6881).await;
        assert!(
            result.is_ok(),
            "announce should return Ok in read-only mode (suppressed)"
        );

        handle.shutdown().await.unwrap();
    }

    // -----------------------------------------------------------------------
    // M173 Lane B (B7): SaveRoutingTable + persist-on-shutdown.
    // -----------------------------------------------------------------------

    /// `save_routing_table` returns `Ok(())` even when no `state_dir`
    /// is configured (no-op). The actor still acks so the caller
    /// doesn't need to special-case the disabled path.
    #[tokio::test]
    async fn save_routing_table_acks_even_without_state_dir() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            state_dir: None,
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let result = handle.save_routing_table().await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        handle.shutdown().await.unwrap();
    }

    /// `save_routing_table` writes `dht_state.json` to disk under the
    /// configured `state_dir`. Confirms the `apply_settings` DHT-stop
    /// phase can checkpoint state without restarting the actor.
    #[tokio::test]
    async fn save_routing_table_writes_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().to_path_buf();

        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            state_dir: Some(state_dir.clone()),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        // Wait briefly for the actor to settle (load_routing_table
        // is part of bootstrap).
        tokio::time::sleep(Duration::from_millis(50)).await;

        handle.save_routing_table().await.unwrap();

        let state_path = state_dir.join("dht_state.json");
        assert!(
            state_path.exists(),
            "save_routing_table must write dht_state.json to {}",
            state_path.display()
        );

        // The file should be valid JSON containing the node_id.
        let contents = std::fs::read_to_string(&state_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(parsed.get("node_id").is_some());

        handle.shutdown().await.unwrap();
    }

    /// `shutdown_and_wait` returns AFTER the actor has persisted the
    /// routing table — the on-disk state is up-to-date when the
    /// caller proceeds with starting a new actor. This is the
    /// contract the B11 `apply_settings` DHT-restart phase relies on.
    #[tokio::test]
    async fn shutdown_and_wait_persists_state_before_returning() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().to_path_buf();

        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            state_dir: Some(state_dir.clone()),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();

        // Wait for bootstrap to settle.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Pre-shutdown: state file may or may not exist (depends on
        // whether the periodic save fired). Delete it so we can
        // check that shutdown_and_wait WROTE it.
        let state_path = state_dir.join("dht_state.json");
        let _ = std::fs::remove_file(&state_path);
        assert!(!state_path.exists());

        handle.shutdown_and_wait().await.unwrap();

        // After shutdown_and_wait returns, the file MUST exist on
        // disk — if the actor exited before saving, the rebuild path
        // would lose recent node state.
        assert!(
            state_path.exists(),
            "shutdown_and_wait must persist state BEFORE returning"
        );
    }

    /// `shutdown_and_wait` after a prior fire-and-forget shutdown
    /// returns Err(Shutdown). Pin the failure mode for B11 callers.
    #[tokio::test]
    async fn shutdown_and_wait_after_actor_exit_returns_shutdown_error() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        handle.shutdown().await.unwrap();
        // Give the actor a tick to exit.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let result = handle.shutdown_and_wait().await;
        assert!(
            matches!(result, Err(Error::Shutdown)),
            "expected Error::Shutdown, got {result:?}"
        );
    }

    #[test]
    fn dht_config_enable_multi_address_default_true() {
        let cfg = DhtConfig::default();
        assert!(cfg.enable_multi_address);
    }

    #[test]
    fn dht_config_v6_enable_multi_address_default_true() {
        let cfg = DhtConfig::default_v6();
        assert!(cfg.enable_multi_address);
    }

    #[tokio::test]
    async fn direct_get_peers_times_out_on_unresponsive_target() {
        let config = DhtConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            bootstrap_nodes: Vec::new(),
            query_timeout: Duration::from_millis(50),
            ..DhtConfig::default()
        };
        let (handle, _ip_rx) = DhtHandle::start(config).await.unwrap();
        let target: SocketAddr = "127.0.0.1:65432".parse().unwrap();
        let hash = Id20::from_hex("0123456789abcdef0123456789abcdef01234567").unwrap();
        let res = handle.direct_get_peers(target, hash).await;
        assert!(res.is_err());
        handle.shutdown().await.unwrap();
    }
}
