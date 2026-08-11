pub mod parse;
pub mod wire;

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use irontide_core::Id20;
use irontide_dht::DhtHandle;
use rand::RngCore;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tracing::debug;

use crate::discovery::SampledHash;
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
const PARALLEL_DIALS: usize = 16;
/// Per-hash wall-clock budget for peer iteration. Successful fetches almost
/// always complete in the first few dials, so a short deadline frees the pool
/// quickly for the next hash.
const FETCH_DEADLINE: Duration = Duration::from_secs(20);
/// Per-peer connect/fetch timeout.
const FETCH_TIMEOUT: Duration = Duration::from_secs(7);
/// Grace period for in-flight fetches during shutdown before they are cancelled.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// The result of one metadata fetch.
enum FetchOutcome {
    /// Metadata verified and persisted to the torrents table.
    Accepted { info_bytes: Vec<u8>, raw_name: String },
}

/// Error from `fetch_one` carrying the dominant failure reason for DB persistence.
struct FetchError {
    reason: anyhow::Error,
    dominant_failure: Option<String>,
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
                    Err(fe) => {
                        stats
                            .fetches_failed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        debug!(error = %fe.reason, %hash, dominant = ?fe.dominant_failure, "metadata fetch failed");
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
                                failure_reason: fe.dominant_failure,
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

/// Fetch and classify one infohash. Acquires a lookup permit only to start the
/// `get_peers` stream (the actor's DhtLookup keeps running in the background and
/// feeds the channel), then releases it so the pool is not blocked by slow peer
/// dialing — `concurrency` bounds in-flight fetches, not `lookup_concurrency`.
/// Peers are dialed in parallel and the first SHA-1-verified result wins.
async fn fetch_one(
    info_hash: Id20,
    handle: DhtHandle,
    tx: mpsc::Sender<TorrentRecord>,
    peer_id: Id20,
    stats: &CrawlStats,
    blocklist: &Blocklist,
    lookup_permits: &Semaphore,
) -> std::result::Result<FetchOutcome, FetchError> {
    let mut peers = {
        let _permit = lookup_permits.acquire().await.context("lookup permit")
            .map_err(|e| FetchError { reason: e, dominant_failure: None })?;
        handle.get_peers(info_hash).await.context("get_peers failed")
            .map_err(|e| FetchError { reason: e, dominant_failure: None })?
    };

    let deadline = tokio::time::Instant::now() + FETCH_DEADLINE;
    let mut tried = 0usize;
    let mut seen_peers: HashSet<SocketAddr> = HashSet::new();
    let mut dialed_ips: HashSet<IpAddr> = HashSet::new();
    let mut any_peers_seen = false;
    let mut failure_counts: HashMap<&'static str, u32> = HashMap::new();

    'outer: while let Some(batch) = peers.recv().await {
        if tokio::time::Instant::now() >= deadline {
            *failure_counts.entry("deadline").or_insert(0) += 1;
            break;
        }
        if !batch.is_empty() {
            any_peers_seen = true;
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
                *failure_counts.entry("deadline").or_insert(0) += 1;
                break;
            }
            let meta = match res {
                Ok(Ok(Ok(m))) => m,
                Ok(Ok(Err(e))) => {
                    let key = classify_error(&e);
                    *failure_counts.entry(key).or_insert(0) += 1;
                    classify_peer_error(&e, stats);
                    debug!(%info_hash, error = %e, "peer metadata fetch failed");
                    continue;
                }
                Ok(Err(_elapsed)) => {
                    *failure_counts.entry("timeout").or_insert(0) += 1;
                    stats
                        .connect_timeout
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    debug!(%info_hash, "peer dial timed out");
                    continue;
                }
                Err(_) => {
                    *failure_counts.entry("other").or_insert(0) += 1;
                    stats
                        .peer_errors_other
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            };

            // SHA-1 must match the sampled infohash; never persist partial data.
            if sha1_info(&meta.info_bytes) != *info_hash.as_bytes() {
                *failure_counts.entry("sha1_mismatch").or_insert(0) += 1;
                debug!(%info_hash, "metadata SHA-1 mismatch, rejected");
                continue;
            }
            stats
                .metadata_verified
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let extracted = match extract_metadata(&meta.info_bytes) {
                Ok(e) => e,
                Err(e) => {
                    *failure_counts.entry("parse_failed").or_insert(0) += 1;
                    debug!(%info_hash, error = %e, "metadata parse failed");
                    continue;
                }
            };

            let now = unix_secs();
            let record = TorrentRecord {
                info_hash: *info_hash.as_bytes(),
                name: extracted.name.clone(),
                size_bytes: Some(extracted.total_size),
                file_count: Some(extracted.file_count),
                first_seen: now,
                last_seen: now,
            };

            tx.send(record).await.context("storage channel closed")
                .map_err(|e| FetchError { reason: e, dominant_failure: None })?;
            stats
                .records_persisted
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(FetchOutcome::Accepted {
                info_bytes: meta.info_bytes,
                raw_name: extracted.name,
            });
        }
    }

    if !any_peers_seen {
        *failure_counts.entry("empty_peers").or_insert(0) += 1;
        stats
            .empty_peers
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    let dominant = failure_counts.iter()
        .max_by_key(|(_, count)| **count)
        .map(|(reason, _)| reason.to_string());

    Err(FetchError {
        reason: anyhow!("no reachable peer yielded verified metadata"),
        dominant_failure: dominant,
    })
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

/// Classify a peer fetch error into a static category string for per-hash tracking.
fn classify_error(e: &anyhow::Error) -> &'static str {
    let msg = e.to_string();
    if msg.contains("timed out") || msg.contains("timeout") {
        "timeout"
    } else if msg.contains("Connection refused") {
        "connect_refused"
    } else if msg.contains("does not support BEP 10") {
        "no_bep10"
    } else if msg.contains("does not advertise ut_metadata") {
        "no_ut_metadata"
    } else if msg.contains("rejected metadata piece") {
        "metadata_rejected"
    } else if msg.contains("SHA-1 mismatch") {
        "sha1_mismatch"
    } else {
        "other"
    }
}

/// Classify a peer fetch error into a diagnostic counter.
fn classify_peer_error(e: &anyhow::Error, stats: &CrawlStats) {
    let msg = e.to_string();
    let rel = std::sync::atomic::Ordering::Relaxed;
    if msg.contains("timed out") || msg.contains("timeout") {
        stats.connect_timeout.fetch_add(1, rel);
    } else if msg.contains("Connection refused") {
        stats.connect_refused.fetch_add(1, rel);
    } else if msg.contains("does not support BEP 10") {
        stats.no_bep10.fetch_add(1, rel);
    } else if msg.contains("does not advertise ut_metadata") {
        stats.no_ut_metadata.fetch_add(1, rel);
    } else if msg.contains("rejected metadata piece") {
        stats.metadata_rejected.fetch_add(1, rel);
    } else if msg.contains("SHA-1 mismatch") {
        stats.sha1_mismatch.fetch_add(1, rel);
    } else {
        stats.peer_errors_other.fetch_add(1, rel);
    }
}
