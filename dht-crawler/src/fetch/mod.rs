pub mod parse;
pub mod wire;

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use gaia_core::Id20;
use gaia_dht::DhtHandle;
use rand::RngCore;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tracing::debug;

use crate::discovery::FetchRequest;
use crate::net::{Blocklist, DeadPeerCache};
use crate::stats::CrawlStats;
use crate::storage::{
    backoff_secs, EMPTY_PEERS_RETRY_SECS, ScannedRecord, ScannedStatus, Storage, TorrentRecord,
};

use parse::extract_metadata;
use wire::{fetch_from_peer, sha1_info};

/// Cap on distinct peers tried per infohash before giving up.
const MAX_PEERS_PER_HASH: usize = 50;
/// How many peers are dialed concurrently per infohash; first verified success wins.
const PARALLEL_DIALS: usize = 16;
/// Per-hash wall-clock budget for peer iteration. Successful fetches almost
/// always complete in the first few dials; the shared dead-peer cache prevents
/// re-dialing known-dead IPs, so a moderate window finds live peers without
/// excessive churn.
const FETCH_DEADLINE: Duration = Duration::from_secs(15);
/// How long to wait for the next `get_peers` batch before giving up. The
/// DhtLookup streams batches into the channel; a slow or empty lookup must not
/// hold a pool slot indefinitely.
const RECV_TIMEOUT: Duration = Duration::from_secs(4);
/// Dials to attempt before concluding a hash is dead (all connect failures).
/// If this many consecutive dials fail with no successful handshake, the fetch
/// aborts early instead of waiting out `FETCH_DEADLINE`.
const EARLY_ABORT_DIALS: usize = 64;
/// Per-peer connect/fetch timeout. 5s (down from 10s) churns dead peers faster
/// now that a larger stream of distinct hashes flows through the pool.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
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

/// Max-heap of pending fetch requests keyed by hash. Announce-derived requests
/// (with a live peer hint) are ordered above sampled ones; among requests for
/// the same hash, the latest/highest occurrence count and most recent hint win.
struct HashQueue {
    heap: BinaryHeap<(bool, u32, Id20)>,
    current: HashMap<Id20, FetchRequest>,
}

impl HashQueue {
    fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            current: HashMap::new(),
        }
    }

    /// Add or upgrade a fetch request for a hash. A request is only admitted
    /// if it reports a strictly higher occurrence count, or if it carries a
    /// peer hint (at least as many occurrences) and the queued entry does not
    /// — announce-derived hashes are more likely to verify, so they preempt an
    /// earlier hint-less entry.
    fn push(&mut self, req: FetchRequest) {
        let cur = self.current.get(&req.hash);
        let admitted = match cur {
            None => true,
            Some(c) => {
                req.occurrences > c.occurrences
                    || (req.peer_hint.is_some() && c.peer_hint.is_none() && req.occurrences >= c.occurrences)
            }
        };
        if !admitted {
            return;
        }
        self.current.insert(req.hash, req.clone());
        let req = self.current.get(&req.hash).expect("just inserted");
        self.heap.push((req.peer_hint.is_some(), req.occurrences, req.hash));
    }

    /// Pop the highest-priority request, skipping stale heap entries.
    fn pop(&mut self) -> Option<FetchRequest> {
        while let Some((hinted, occ, h)) = self.heap.pop() {
            let cur = self.current.get(&h).cloned();
            if let Some(cur) = cur {
                if cur.occurrences == occ && cur.peer_hint.is_some() == hinted {
                    self.current.remove(&h);
                    return Some(cur);
                }
            }
        }
        None
    }

    /// Number of distinct hashes currently queued and not yet fetched.
    fn depth(&self) -> usize {
        self.current.len()
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
    /// Shared cross-instance state (dead-peer cache, seen-set).
    pub shared: crate::redis::SharedState,
}

/// Consume popularity-ordered infohashes from `rx`, fetch verified metadata
/// for each, classify it, and forward accepted torrents to `tx`. Concurrency
/// is bounded by `cfg.concurrency` in-flight fetches and
/// `cfg.lookup_concurrency` concurrent DHT `get_peers` lookups.
pub async fn run_fetcher(
    mut rx: mpsc::Receiver<FetchRequest>,
    tx: mpsc::Sender<TorrentRecord>,
    handle: DhtHandle,
    storage: Storage,
    cfg: FetcherConfig,
    stats: Arc<CrawlStats>,
) {
    let peer_id = random_peer_id();
    let lookup_permits = Arc::new(Semaphore::new(cfg.lookup_concurrency.max(1)));
    let in_flight: Arc<Mutex<HashSet<Id20>>> = Arc::new(Mutex::new(HashSet::new()));
    let dead_peers: Arc<Mutex<DeadPeerCache>> = Arc::new(Mutex::new(DeadPeerCache::new(2, 600)));
    let mut queue = HashQueue::new();
    let mut tasks = JoinSet::new();
    let max = cfg.concurrency.max(1);
    let mut rx_closed = false;

    loop {
        // Refill free slots with the highest-priority ready hashes.
        while tasks.len() < max {
            let Some(req) = queue.pop() else { break };
            let hash = req.hash;
            if in_flight.lock().unwrap().contains(&hash) {
                continue;
            }
            // Batch-check a window of candidates in one query: admission
            // checks are redundant with the sampler's emit-time check, so a
            // single `IN` query over a chunk is far cheaper than N point
            // lookups as the unique stream grows.
            let mut chunk: Vec<FetchRequest> = Vec::with_capacity(64);
            chunk.push(req);
            while chunk.len() < 64 {
                let Some(h) = queue.pop() else { break };
                if !in_flight.lock().unwrap().contains(&h.hash) {
                    chunk.push(h);
                }
            }
            let now = unix_secs();
            let chunk_bytes: Vec<[u8; 20]> = chunk.iter().map(|r| *r.hash.as_bytes()).collect();
            let blocked = storage
                .scan_blocked_batch(&chunk_bytes, now)
                .unwrap_or_default();
            let blocked: std::collections::HashSet<[u8; 20]> = blocked.into_iter().collect();
            for req in chunk {
                if blocked.contains(req.hash.as_bytes()) {
                    continue;
                }
                in_flight.lock().unwrap().insert(req.hash);
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
                let dead_peers = dead_peers.clone();
                let shared = cfg.shared.clone();
                let peer_hint = req.peer_hint;
                tasks.spawn(async move {
                    let outcome = fetch_one(
                        req.hash,
                        peer_hint,
                        handle,
                        tx,
                        peer_id,
                        &stats,
                        &blocklist,
                        &permits,
                        &dead_peers,
                        &shared,
                    )
                    .await;
                    match outcome {
                        Ok(FetchOutcome::Accepted { info_bytes, raw_name }) => {
                        let _ = storage.record_scanned(&ScannedRecord {
                            info_hash: *req.hash.as_bytes(),
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
                        debug!(error = %fe.reason, %req.hash, dominant = ?fe.dominant_failure, "metadata fetch failed");
                        let attempts = storage
                            .scan_status(req.hash.as_bytes())
                            .ok()
                            .flatten()
                            .map_or(1, |s| match s {
                                ScannedStatus::Failed { attempts, .. } => attempts + 1,
                                _ => 1,
                            });
                        let now = unix_secs();
                        let delay = if fe.dominant_failure.as_deref() == Some("empty_peers") {
                            EMPTY_PEERS_RETRY_SECS
                        } else {
                            backoff_secs(attempts)
                        };
                        let _ = storage.record_scanned(&ScannedRecord {
                            info_hash: *req.hash.as_bytes(),
                            status: ScannedStatus::Failed {
                                attempts,
                                next_attempt: now + delay,
                                failure_reason: fe.dominant_failure,
                            },
                            info_bytes: None,
                            raw_name: None,
                            last_attempt: now,
                        });
                    }
                }
                in_flight.lock().unwrap().remove(&req.hash);
            });
            }
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

        // Publish pipeline depth snapshots for the stats loop.
        stats
            .fetch_in_flight
            .store(tasks.len() as u64, std::sync::atomic::Ordering::Relaxed);
        stats
            .queue_depth
            .store(queue.depth() as u64, std::sync::atomic::Ordering::Relaxed);
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
#[allow(clippy::too_many_arguments)]
async fn fetch_one(
    info_hash: Id20,
    peer_hint: Option<SocketAddr>,
    handle: DhtHandle,
    tx: mpsc::Sender<TorrentRecord>,
    peer_id: Id20,
    stats: &CrawlStats,
    blocklist: &Blocklist,
    lookup_permits: &Semaphore,
    dead_peers: &Arc<Mutex<DeadPeerCache>>,
    shared: &crate::redis::SharedState,
) -> std::result::Result<FetchOutcome, FetchError> {
    let deadline = tokio::time::Instant::now() + FETCH_DEADLINE;
    let mut tried = 0usize;
    let mut seen_peers: HashSet<SocketAddr> = HashSet::new();
    let mut dialed_ips: HashSet<IpAddr> = HashSet::new();
    let mut any_peers_seen = false;
    let mut failure_counts: HashMap<&'static str, u32> = HashMap::new();
    // Counts dials that failed to connect or handshake. If this reaches
    // EARLY_ABORT_DIALS before any successful handshake, the hash is dead.
    let mut consecutive_connect_failures = 0usize;
    let mut any_handshake = false;

    // Passive-intake fast path: the hash came from an inbound announce_peer,
    // so we already know a peer that announced it. Dial that peer directly
    // before spending a `get_peers` lookup. If it verifies, we win without any
    // discovery traffic — this is the whole point of the announce-first path.
    if let Some(hint) = peer_hint {
        let now = unix_secs();
        if !blocklist.contains(hint.ip())
            && !dead_peers.lock().unwrap().is_dead(hint.ip(), now)
            && !shared.dead_contains(hint.ip()).await
            && seen_peers.insert(hint)
            && dialed_ips.insert(hint.ip())
        {
            tried += 1;
            any_peers_seen = true;
            let result =
                tokio::time::timeout(FETCH_TIMEOUT, fetch_from_peer(hint, info_hash, peer_id))
                    .await;
            if let Ok(Ok(meta)) = result {
                if sha1_info(&meta.info_bytes) == *info_hash.as_bytes() {
                    stats
                        .metadata_verified
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let extracted = extract_metadata(&meta.info_bytes).ok();
                    if let Some(extracted) = extracted {
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
                } else {
                    *failure_counts.entry("sha1_mismatch").or_insert(0) += 1;
                }
            }
        }
    }

    let mut peers = {
        let _permit = lookup_permits.acquire().await.context("lookup permit")
            .map_err(|e| FetchError { reason: e, dominant_failure: None })?;
        handle.get_peers(info_hash).await.context("get_peers failed")
            .map_err(|e| FetchError { reason: e, dominant_failure: None })?
    };

    'outer: loop {
        // Bound the wait for the next peer batch so a slow/empty get_peers
        // lookup cannot hold a pool slot indefinitely.
        let batch = match tokio::time::timeout(RECV_TIMEOUT, peers.recv()).await {
            Ok(Some(batch)) => batch,
            Ok(None) => break,            // lookup exhausted
            Err(_elapsed) => break,       // no batch arrived in time → stall
        };
        if tokio::time::Instant::now() >= deadline {
            *failure_counts.entry("deadline").or_insert(0) += 1;
            break;
        }
        if !batch.is_empty() {
            any_peers_seen = true;
        }

        let mut candidates: Vec<SocketAddr> = Vec::with_capacity(PARALLEL_DIALS);
        let now = unix_secs();
        dead_peers.lock().unwrap().prune(now);
        for peer in batch {
            if tried >= MAX_PEERS_PER_HASH {
                break 'outer;
            }
            if blocklist.contains(peer.ip()) {
                continue;
            }
            if dead_peers.lock().unwrap().is_dead(peer.ip(), now) {
                continue;
            }
            // Fleet-wide dead check (best-effort; Redis failure allows dialing).
            if shared.dead_contains(peer.ip()).await {
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
                let result =
                    tokio::time::timeout(FETCH_TIMEOUT, fetch_from_peer(peer, info_hash, peer_id))
                        .await;
                (peer, result)
            });
        }
        while let Some(res) = dials.join_next().await {
            if tokio::time::Instant::now() >= deadline {
                *failure_counts.entry("deadline").or_insert(0) += 1;
                break;
            }
            let (peer, inner) = match res {
                Ok(v) => v,
                Err(_) => {
                    *failure_counts.entry("other").or_insert(0) += 1;
                    stats
                        .peer_errors_other
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            };
            let now = unix_secs();
            let meta = match inner {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => {
                    // The TCP connect + handshake succeeded; this peer is
                    // reachable even though metadata failed, so the hash is
                    // not dead. Reset the early-abort counter.
                    consecutive_connect_failures = 0;
                    any_handshake = true;
                    let key = classify_error(&e);
                    *failure_counts.entry(key).or_insert(0) += 1;
                    classify_peer_error(&e, stats);
                    debug!(%info_hash, error = %e, "peer metadata fetch failed");
                    continue;
                }
                Err(_elapsed) => {
                    *failure_counts.entry("timeout").or_insert(0) += 1;
                    stats
                        .connect_timeout
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    debug!(%info_hash, "peer dial timed out");
                    let became_dead = dead_peers.lock().unwrap().record_failure(peer.ip(), now);
                    if became_dead {
                        // Flag fleet-wide so other instances skip it too.
                        shared.dead_add(peer.ip(), 600).await;
                    }
                    consecutive_connect_failures += 1;
                    if !any_handshake && consecutive_connect_failures >= EARLY_ABORT_DIALS {
                        *failure_counts.entry("early_abort").or_insert(0) += 1;
                        stats
                            .early_abort
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        break 'outer;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn hash(n: u8) -> Id20 {
        Id20([n; 20])
    }

    fn addr(n: u8) -> SocketAddr {
        format!("10.0.0.{n}:6881").parse().unwrap()
    }

    #[test]
    fn queue_pops_hinted_request_first() {
        let mut q = HashQueue::new();
        // A sampled request with many occurrences...
        q.push(FetchRequest {
            hash: hash(1),
            occurrences: 10,
            peer_hint: None,
        });
        // ...and a hinted announce with a single occurrence.
        q.push(FetchRequest {
            hash: hash(2),
            occurrences: 1,
            peer_hint: Some(addr(2)),
        });
        let first = q.pop().expect("a request");
        assert_eq!(first.hash, hash(2), "hinted announce must pop before sampled");
        assert_eq!(first.peer_hint, Some(addr(2)));
    }

    #[test]
    fn queue_hint_upgrades_existing_entry() {
        let mut q = HashQueue::new();
        q.push(FetchRequest {
            hash: hash(1),
            occurrences: 5,
            peer_hint: None,
        });
        // A later announce for the same hash adds the hint.
        q.push(FetchRequest {
            hash: hash(1),
            occurrences: 5,
            peer_hint: Some(addr(1)),
        });
        let got = q.pop().expect("a request");
        assert_eq!(got.hash, hash(1));
        assert!(got.peer_hint.is_some(), "hint must upgrade the entry");
    }

    #[test]
    fn queue_ignores_stale_lower_occurrence() {
        let mut q = HashQueue::new();
        q.push(FetchRequest {
            hash: hash(1),
            occurrences: 7,
            peer_hint: None,
        });
        // A lower occurrence report must be ignored.
        q.push(FetchRequest {
            hash: hash(1),
            occurrences: 3,
            peer_hint: Some(addr(1)),
        });
        let got = q.pop().expect("a request");
        assert_eq!(got.occurrences, 7);
    }
}
