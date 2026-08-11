pub mod parse;
pub mod wire;

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use irontide_core::Id20;
use irontide_dht::DhtHandle;
use rand::RngCore;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tracing::debug;

use crate::dht::SampledHash;
use crate::filter::MediaFilter;
use crate::net::Blocklist;
use crate::stats::CrawlStats;
use crate::storage::{
    backoff_secs, ScannedRecord, ScannedStatus, Storage, TorrentRecord,
};

use parse::extract_metadata;
use wire::{fetch_from_peer, sha1_info};

/// Cap on distinct peers tried per infohash before giving up.
const MAX_PEERS_PER_HASH: usize = 50;
/// How many peers are dialed concurrently per infohash; first verified success wins.
const PARALLEL_DIALS: usize = 8;
/// Per-hash wall-clock budget for peer iteration.
const FETCH_DEADLINE: Duration = Duration::from_secs(90);
/// Per-peer connect/fetch timeout.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Grace period for in-flight fetches during shutdown before they are cancelled.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// The result of one metadata fetch.
enum FetchOutcome {
    /// Metadata verified, classified, and persisted to the torrents table.
    Accepted { info_bytes: Vec<u8>, raw_name: String },
    /// Metadata verified but filtered out (not movie/TV).
    Skipped { info_bytes: Vec<u8>, raw_name: String },
}

/// Max-heap of pending hashes keyed by their reported popularity, with
/// duplicate/stale entries suppressed via the `current` map.
struct HashQueue {
    heap: BinaryHeap<(u32, Id20)>,
    current: HashMap<Id20, u32>,
}

impl HashQueue {
    fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            current: HashMap::new(),
        }
    }

    /// Add a popularity report, upgrading priority if it is newer and higher.
    fn push(&mut self, sh: SampledHash) {
        let cur = self.current.entry(sh.hash).or_insert(0);
        if sh.occurrences <= *cur {
            return; // stale or duplicate report
        }
        *cur = sh.occurrences;
        self.heap.push((sh.occurrences, sh.hash));
    }

    /// Pop the highest-priority hash, skipping stale heap entries.
    fn pop(&mut self) -> Option<Id20> {
        while let Some((occ, h)) = self.heap.pop() {
            if self.current.get(&h) == Some(&occ) {
                self.current.remove(&h);
                return Some(h);
            }
        }
        None
    }
}

/// Fetcher pool settings.
pub struct FetcherConfig {
    /// Maximum concurrent in-flight metadata fetches.
    pub concurrency: usize,
    /// Maximum concurrent DHT `get_peers` lookups.
    pub lookup_concurrency: usize,
    /// Peer/IP blocklist applied before dialing.
    pub blocklist: Arc<Blocklist>,
}

/// Consume popularity-ordered infohashes from `rx`, fetch verified metadata
/// for each, classify it, and forward accepted torrents to `tx`. Concurrency
/// is bounded by `cfg.concurrency` in-flight fetches and
/// `cfg.lookup_concurrency` concurrent DHT `get_peers` lookups.
pub async fn run_fetcher(
    mut rx: mpsc::Receiver<SampledHash>,
    tx: mpsc::Sender<TorrentRecord>,
    handle: DhtHandle,
    storage: Storage,
    cfg: FetcherConfig,
    stats: Arc<CrawlStats>,
) {
    let peer_id = random_peer_id();
    let lookup_permits = Arc::new(Semaphore::new(cfg.lookup_concurrency.max(1)));
    let in_flight: Arc<Mutex<HashSet<Id20>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut queue = HashQueue::new();
    let mut tasks = JoinSet::new();
    let max = cfg.concurrency.max(1);
    let mut rx_closed = false;

    loop {
        // Refill free slots with the highest-priority ready hashes.
        while tasks.len() < max {
            let Some(hash) = queue.pop() else { break };
            if in_flight.lock().unwrap().contains(&hash) {
                continue;
            }
            if storage.scan_blocked(hash.as_bytes(), unix_secs()).unwrap_or(false) {
                continue;
            }
            in_flight.lock().unwrap().insert(hash);
            stats
                .fetches_attempted
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let handle = handle.clone();
            let tx = tx.clone();
            let storage = storage.clone();
            let stats = stats.clone();
            let blocklist = cfg.blocklist.clone();
            let permits = lookup_permits.clone();
            let in_flight = in_flight.clone();
            tasks.spawn(async move {
                let outcome = fetch_one(hash, handle, tx, peer_id, &stats, &blocklist, &permits)
                    .await;
                match outcome {
                    Ok(FetchOutcome::Accepted { info_bytes, raw_name }) => {
                        let _ = storage.record_scanned(&ScannedRecord {
                            info_hash: *hash.as_bytes(),
                            status: ScannedStatus::Ok,
                            info_bytes: Some(info_bytes),
                            raw_name: Some(raw_name),
                            last_attempt: unix_secs(),
                        });
                    }
                    Ok(FetchOutcome::Skipped { info_bytes, raw_name }) => {
                        stats
                            .filtered_skip
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = storage.record_scanned(&ScannedRecord {
                            info_hash: *hash.as_bytes(),
                            status: ScannedStatus::Skipped,
                            info_bytes: Some(info_bytes),
                            raw_name: Some(raw_name),
                            last_attempt: unix_secs(),
                        });
                    }
                    Err(e) => {
                        stats
                            .fetches_failed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        debug!(error = %e, %hash, "metadata fetch failed");
                        let attempts = storage
                            .scan_status(hash.as_bytes())
                            .ok()
                            .flatten()
                            .map_or(1, |s| match s {
                                ScannedStatus::Failed { attempts, .. } => attempts + 1,
                                _ => 1,
                            });
                        let now = unix_secs();
                        let _ = storage.record_scanned(&ScannedRecord {
                            info_hash: *hash.as_bytes(),
                            status: ScannedStatus::Failed {
                                attempts,
                                next_attempt: now + backoff_secs(attempts),
                            },
                            info_bytes: None,
                            raw_name: None,
                            last_attempt: now,
                        });
                    }
                }
                in_flight.lock().unwrap().remove(&hash);
            });
        }

        if rx_closed && tasks.is_empty() {
            break;
        }

        tokio::select! {
            msg = rx.recv(), if !rx_closed => {
                match msg {
                    Some(sh) => queue.push(sh),
                    None => rx_closed = true,
                }
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }

    // Drain in-flight fetches before returning (graceful shutdown), but bound
    // the wait so a slow peer cannot stall shutdown indefinitely.
    let drain_deadline = tokio::time::Instant::now() + SHUTDOWN_DRAIN;
    while !tasks.is_empty() && tokio::time::Instant::now() < drain_deadline {
        let _ = tokio::time::timeout_at(drain_deadline, tasks.join_next()).await;
    }
    tasks.abort_all();
}

/// Fetch and classify one infohash. Holds a lookup permit (bounding concurrent
/// DHT lookups) for the whole call. Peers are dialed in parallel and the first
/// SHA-1-verified, classified result wins.
async fn fetch_one(
    info_hash: Id20,
    handle: DhtHandle,
    tx: mpsc::Sender<TorrentRecord>,
    peer_id: Id20,
    stats: &CrawlStats,
    blocklist: &Blocklist,
    lookup_permits: &Semaphore,
) -> Result<FetchOutcome> {
    let _permit = lookup_permits.acquire().await.context("lookup permit")?;
    let mut peers = handle.get_peers(info_hash).await.context("get_peers failed")?;

    let deadline = tokio::time::Instant::now() + FETCH_DEADLINE;
    let mut tried = 0usize;
    let mut seen_peers: HashSet<SocketAddr> = HashSet::new();
    let mut dialed_ips: HashSet<IpAddr> = HashSet::new();

    'outer: while let Some(batch) = peers.recv().await {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        let mut candidates: Vec<SocketAddr> = Vec::with_capacity(PARALLEL_DIALS);
        for peer in batch {
            if tried >= MAX_PEERS_PER_HASH {
                break 'outer;
            }
            if blocklist.contains(peer.ip()) {
                continue;
            }
            if !seen_peers.insert(peer) || !dialed_ips.insert(peer.ip()) {
                continue;
            }
            candidates.push(peer);
            tried += 1;
            if candidates.len() >= PARALLEL_DIALS {
                break;
            }
        }
        if candidates.is_empty() {
            continue;
        }

        let mut dials = JoinSet::new();
        for peer in candidates {
            dials.spawn(async move {
                tokio::time::timeout(FETCH_TIMEOUT, fetch_from_peer(peer, info_hash, peer_id)).await
            });
        }
        while let Some(res) = dials.join_next().await {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let meta = match res {
                Ok(Ok(Ok(m))) => m,
                _ => continue,
            };

            // SHA-1 must match the sampled infohash; never persist partial data.
            if sha1_info(&meta.info_bytes) != *info_hash.as_bytes() {
                debug!(%info_hash, "metadata SHA-1 mismatch, rejected");
                continue;
            }
            stats
                .metadata_verified
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let extracted = match extract_metadata(&meta.info_bytes) {
                Ok(e) => e,
                Err(e) => {
                    debug!(%info_hash, error = %e, "metadata parse failed");
                    continue;
                }
            };

            let filter = MediaFilter;
            let Some(class) = filter
                .classify(&extracted.name)
                .or_else(|| filter.classify_by_files(&extracted.files))
            else {
                return Ok(FetchOutcome::Skipped {
                    info_bytes: meta.info_bytes,
                    raw_name: extracted.name,
                });
            };

            let now = unix_secs();
            let record = TorrentRecord {
                info_hash: *info_hash.as_bytes(),
                name: extracted.name.clone(),
                category: class.category,
                title: class.title,
                year: class.year,
                season: class.season,
                episode: class.episode,
                size_bytes: Some(extracted.total_size),
                file_count: Some(extracted.file_count),
                first_seen: now,
                last_seen: now,
            };

            tx.send(record).await.context("storage channel closed")?;
            stats
                .records_persisted
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(FetchOutcome::Accepted {
                info_bytes: meta.info_bytes,
                raw_name: extracted.name,
            });
        }
    }

    Err(anyhow!("no reachable peer yielded verified metadata"))
}

fn random_peer_id() -> Id20 {
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    Id20(bytes)
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
