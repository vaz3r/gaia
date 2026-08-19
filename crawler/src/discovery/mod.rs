mod liveness;
mod sampler;

pub use liveness::{LivenessConfig, LivenessCounter, RecordOutcome};
pub use sampler::{random_id20, Sampler, SamplerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gaia_core::{AddressFamily, Id20};
use gaia_dht::{DhtConfig, DhtHandle};
use rand::RngCore;
use tokio_util::sync::CancellationToken;

use crate::cli::RunArgs;

/// A hash requested for metadata fetch, optionally carrying a live peer hint.
///
/// `Some(peer_hint)` means an inbound `announce_peer` proved the hash is live
/// and told us exactly which peer to dial — the fetch pipeline tries that peer
/// directly before falling back to a `get_peers` lookup. This is the
/// passive-intake discovery path (bitmagnet's `PutHash` pattern): announced
/// hashes have a dramatically higher fetch-success rate than sampled ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSource {
    /// Discovered via BEP 51 sampling.
    Sampled,
    /// Inbound `announce_peer` carrying a live dial hint.
    Announced,
    /// Inbound `get_peers` — someone actively seeking the hash.
    LookedUp,
    /// Resolved via public trackers (BEP 15 UDP announce).
    Tracker,
    /// Actively re-fetched by the retry worker (an already-failed hash whose
    /// next_attempt has passed and which is under its class retry cap).
    Retried,
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub hash: Id20,
    pub occurrences: u32,
    pub peer_hint: Option<SocketAddr>,
    pub source: FetchSource,
    /// A DHT node known to hold this hash (the BEP 51 reporting node). The
    /// fetch seeds its get_peers lookup with this node so the lookup reaches
    /// the node that proved it has the hash — recovering the ~49% empty_peers
    /// failures where the keyspace-closest lookup misses the reporting node.
    pub lookup_seed: Option<SocketAddr>,
    pub dht_handle: Option<gaia_dht::DhtHandle>,
}

/// Start the DHT actor for instance `i` (0-based), bound to `port + i`. No file
/// persistence: the node ID is loaded from / persisted to Redis, and routing
/// state lives in the shared Redis node pool (the crawler seeds from it at
/// startup and persists back on shutdown). `extra_seeds` are `host:port`
/// bootstrap nodes (e.g. from the shared pool or the primary's warm table).
pub async fn start_dht(
    args: &RunArgs,
    instance: usize,
    extra_seeds: Vec<String>,
    shared: &crate::redis::SharedState,
) -> Result<DhtHandle> {
    let port = args.port.saturating_add(instance as u16);
    let bind_addr = if args.ipv6 {
        std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port))
    } else {
        std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, port))
    };
    let mut bootstrap = args.bootstrap.clone();
    bootstrap.extend(extra_seeds);
    let own_id = load_or_create_node_id_redis(instance, shared).await;
    let dht = DhtConfig {
        bind_addr,
        bootstrap_nodes: bootstrap,
        // No file persistence — Redis is the only state store.
        state_dir: None,
        address_family: if args.ipv6 {
            AddressFamily::V6
        } else {
            AddressFamily::V4
        },
        queries_per_second: args.effective_qps(),
        max_routing_nodes: args.effective_max_nodes(),
        query_timeout: Duration::from_secs(args.effective_query_timeout()),
        restrict_routing_ips: !args.no_restrict_ips,
        own_id,
        ..DhtConfig::default()
    };
    let (handle, _ip) = DhtHandle::start(dht).await?;
    Ok(handle)
}

/// Load or create a stable node ID for instance `i`, persisted in Redis (no
/// files). A stable ID lets the DHT node build reputation over restarts: peers
/// keep routing `announce_peer`/`get_peers` queries to a well-known ID.
async fn load_or_create_node_id_redis(
    instance: usize,
    shared: &crate::redis::SharedState,
) -> Option<gaia_core::Id20> {
    if let Some(hex) = shared.node_id_get(instance).await {
        if let Ok(id) = gaia_core::Id20::from_hex(&hex) {
            return Some(id);
        }
    }
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = gaia_core::Id20(bytes);
    shared.node_id_set(instance, &id.to_hex()).await;
    Some(id)
}

/// Continuously grow the routing table (bitmagnet's `getNodesForFindNode` +
/// `runFindNode` pattern, adapted to cycle the whole table). Each tick:
/// 1. `find_node` a batch of table nodes with a FRESH random target per query —
///    response nodes are injected back into the routing table by the actor.
///    Cycling the whole table (via a rotating cursor) both refreshes existing
///    entries and pulls their neighbors in, so the table climbs toward capacity.
/// 2. Issue a random-target `get_peers` walk (dropped reply channel → DhtLookup
///    fast-exits after a couple responses) to discover nodes in untouched
///    keyspace regions.
///
/// NOTE: each `find_node` must use its OWN fresh target. A shared target per
/// tick makes all batch queries return the same ~8 closest nodes (redundant),
/// the bug that kept node inflow far below bitmagnet-scale.
pub async fn grow_routing(
    handle: DhtHandle,
    interval: Duration,
    shutdown: CancellationToken,
    sought_rx: tokio::sync::watch::Receiver<Id20>,
) {
    // Continuous random-target get_peers walkers. The reply channel MUST be
    // held open for the DhtLookup to keep traversing: dropping it makes the
    // walk wind down after ~2 responses, so distant keyspace is never ingested
    // and the routing table can't grow past the bootstrap pool — the
    // samplable-node supply ceiling that caps unique-hash discovery. Each
    // walker repeatedly launches a deep (256-node) walk and drains its batches
    // so the actor injects every discovered closer node into the routing table.
    let mut walkers = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let handle = handle.clone();
        let shutdown = shutdown.clone();
        walkers.spawn(async move {
            loop {
                if shutdown.is_cancelled() {
                    return;
                }
                let mut bytes = [0u8; 20];
                rand::thread_rng().fill_bytes(&mut bytes);
                if let Ok(mut rx) = handle.get_peers(Id20(bytes)).await {
                    let drain_deadline =
                        tokio::time::Instant::now() + Duration::from_secs(2);
                    while tokio::time::Instant::now() < drain_deadline {
                        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                            Ok(Some(_batch)) => {}
                            Ok(None) => break,
                            Err(_) => {}
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }

    // Bitmagnet's `runFindNode`: every tick, find_node the OLDEST nodes in the
    // table using the SHARED rotating sought target (not a random target per
    // query). Oldest-first re-queries the longest-unattended nodes so their
    // stale links are refreshed and their fresh response nodes keep the table
    // populated. Using the shared sought target makes find_node responses
    // cluster in the region the sampler is currently scanning, so newly
    // discovered nodes are directly sampling-relevant. Nodes that fail are
    // dropped from the routing table (bitmagnet's DropNode) so dead/read-only
    // nodes stop burning discovery budget.
    let mut cursor: usize = 0;
    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let nodes = handle.get_oldest_nodes(256).await;
        if !nodes.is_empty() {
            let batch = 256usize;
            let sought = *sought_rx.borrow();
            let mut tasks = tokio::task::JoinSet::new();
            for k in 0..batch.min(nodes.len()) {
                if shutdown.is_cancelled() {
                    return;
                }
                let idx = (cursor + k) % nodes.len();
                let addr = nodes[idx].1;
                let handle = handle.clone();
                let sought = sought;
                tasks.spawn(async move {
                    if handle.find_node(addr, sought).await.is_err() {
                        // Bitmagnet's DropNode: a node that fails find_node can't
                        // help discovery or sampling. Remove it so the table
                        // keeps room for fresh, reachable nodes.
                        handle.remove_node(addr).await;
                    }
                });
            }
            while tasks.join_next().await.is_some() {}
            cursor = (cursor + batch) % nodes.len();
        }

        tokio::time::sleep(interval).await;
    }
}

/// Passive-intake loop: subscribe to the DHT node's inbound events and forward
/// every `announce_peer` to the fetch pipeline with its live peer as a dial
/// hint. Announced hashes are live by construction (a peer is announcing them
/// right now), so they go straight to a peer dial — no `get_peers` discovery.
///
/// `min_peer_port` skips peers whose port is a well-known non-torrent port
/// (e.g. 6881 is fine; 80/443 are usually NAT artifacts). Best-effort: events
/// are dropped on Redis dedup collisions, which is the desired behaviour (the
/// shared seen-set is the single source of "already fetched" truth).
pub async fn run_passive_intake(
    handle: DhtHandle,
    emit: tokio::sync::mpsc::Sender<FetchRequest>,
    stats: Arc<crate::stats::CrawlStats>,
    shared: crate::redis::SharedState,
    shutdown: CancellationToken,
) {
    let mut events = handle.subscribe();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            event = events.recv() => {
                let Ok(event) = event else { return };
                match event {
                    gaia_dht::DhtEvent::Announced { info_hash, peer_addr } => {
                        // Dedup only against OTHER announce fetches, not the
                        // sampler's blind fetches: an announce carries a live
                        // peer hint and converts far higher, so it must never
                        // be dropped because the sampler already tried (and
                        // probably failed on) this hash.
                        if shared.announced_contains(info_hash.as_bytes()).await {
                            stats
                                .announces_deduped_redis
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        }
                        stats
                            .hashes_announced
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        stats
                            .announces_emitted
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if emit
                            .send(FetchRequest {
                                hash: info_hash,
                                occurrences: 1,
                                peer_hint: Some(peer_addr),
                                source: FetchSource::Announced,
                                lookup_seed: None,
                                dht_handle: Some(handle.clone()),
                            })
                            .await
                            .is_err()
                        {
                            return; // pipeline closed → shutdown
                        }
                        shared.announced_add(info_hash.as_bytes()).await;
                    }
                    gaia_dht::DhtEvent::LookedUp { info_hash, from_addr } => {
                        // Someone is actively seeking this hash right now — a
                        // live signal with far more volume than announce_peer.
                        // The seeking client is itself a live peer holding the
                        // metadata (it must have the infohash to seek peers for
                        // it).
                        //
                        // NOTE: we do NOT dial `from_addr` directly. `from_addr`
                        // is a DHT routing node (typically port 6881) that is
                        // forwarding the get_peers query — pure DHT
                        // infrastructure, not a BitTorrent client, so it does NOT
                        // serve ut_metadata (measured: 8,517 looked-up hashes
                        // emitted with peer_hint=from_addr converted ZERO). The
                        // correct path is a keyspace-convergent DHT get_peers
                        // toward the hash, which reaches the ACTUAL clients
                        // holding the metadata. So we pass NO peer_hint (letting
                        // fetch_one run the full get_peers + dial) and seed the
                        // lookup from the DHT node that proved the hash is live.
                        if shared.looked_up_contains(info_hash.as_bytes()).await {
                            stats
                                .lookups_deduped_redis
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        }
                        stats
                            .lookups_emitted
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if emit
                            .send(FetchRequest {
                                hash: info_hash,
                                occurrences: 1,
                                peer_hint: None,
                                source: FetchSource::LookedUp,
                                lookup_seed: Some(from_addr),
                                dht_handle: Some(handle.clone()),
                            })
                            .await
                            .is_err()
                        {
                            return; // pipeline closed → shutdown
                        }
                        shared.looked_up_add(info_hash.as_bytes()).await;
                    }
                }
            }
        }
    }
}
