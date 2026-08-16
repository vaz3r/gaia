mod liveness;
mod sampler;

pub use liveness::{LivenessConfig, LivenessCounter, RecordOutcome};
pub use sampler::{Sampler, SamplerConfig};
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

/// Continuously grow the routing table by issuing `get_peers` on random 20-byte
/// targets. Each lookup walks toward the target and injects discovered nodes
/// into the routing table, so the table climbs toward `--max-nodes` throughout
/// the crawl rather than stalling after the startup warmup.
///
/// The grower drops each lookup's reply channel immediately so the DhtLookup
/// fast-exits after a couple responses — keeping per-lookup memory bounded
/// even at 8 instances × continuous growth (a held-open channel accumulated
/// ~130 MB/hr).
pub async fn grow_routing(
    handle: DhtHandle,
    interval: Duration,
    shutdown: CancellationToken,
) {
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        let target = Id20(bytes);
        // Drop the reply channel immediately so the DhtLookup fast-exits after
        // a couple responses (the leak-safe behavior). A held-open channel
        // makes each lookup walk deeper and retain more state — with 8
        // instances × continuous growers that accumulated ~130 MB/hr.
        let _ = handle.get_peers(target).await;
        // Grow continuously at `interval` (250ms from crawler.rs): the routing
        // table is the binding constraint on unique discovery, so we keep the
        // table climbing toward --max-nodes at all times. The leak fixes
        // (shared sampler maps, bounded announce_tokens, fast-exit lookups)
        // keep memory flat even at a sustained grow rate.
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
                        // it), so dial it directly for the ut_metadata exchange
                        // instead of burning a blind DHT lookup that mostly
                        // ends in empty_peers. Non-routable (NAT'd) addresses
                        // are filtered at dial time by the fetch path.
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
                                peer_hint: Some(from_addr),
                                source: FetchSource::LookedUp,
                                lookup_seed: None,
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
