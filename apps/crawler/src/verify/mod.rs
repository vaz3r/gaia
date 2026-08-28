pub mod fetch_pool;
pub mod peer_cache;
pub mod peer_source;
#[allow(clippy::module_inception)]
pub mod verify;
pub mod wire;

use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::storage::batch_writer::BatchWriter;
use crate::verify::fetch_pool::{FetchParams, VerifyResult, verify_infohash};
use crate::verify::peer_cache::PeerCache;
use crate::verify::verify::check;
use librqbit_utp::UtpSocketUdp;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc};

pub struct AnnouncePeerCache {
    inner: dashmap::DashMap<Infohash, (SocketAddr, Instant)>,
    ttl: Duration,
    max_entries: usize,
}

impl AnnouncePeerCache {
    pub fn new(ttl: Duration, max_entries: usize, initial_capacity: usize, shards: usize) -> Self {
        AnnouncePeerCache {
            inner: dashmap::DashMap::with_capacity_and_shard_amount(
                initial_capacity.max(1),
                shards.max(1),
            ),
            ttl,
            max_entries,
        }
    }

    pub fn insert(&self, ih: Infohash, addr: SocketAddr) {
        self.inner.insert(ih, (addr, Instant::now()));
        self.enforce_bound();
    }

    pub fn get(&self, ih: &Infohash) -> Option<SocketAddr> {
        self.inner.get(ih).and_then(|entry| {
            if entry.1.elapsed() < self.ttl {
                Some(entry.0)
            } else {
                drop(entry);
                self.inner.remove(ih);
                None
            }
        })
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    fn enforce_bound(&self) {
        if self.inner.len() <= self.max_entries {
            return;
        }
        let now = Instant::now();
        self.inner.retain(|_, (_, ts)| now.duration_since(*ts) < self.ttl);
    }
}

pub struct VerifyConfig {
    pub pipeline_limit: usize,
    pub fetch_limit: usize,
    pub params: FetchParams,
}

/// Bounds concurrent TCP+uTP connects per peer IP so a small set of high-value
/// multi-port seedboxes is not hammered (prevents librqbit_utp "too many
/// concurrent connections" floods and wasted connect attempts).
pub struct ConnLimiter {
    inner: dashmap::DashMap<std::net::IpAddr, Arc<tokio::sync::Semaphore>>,
    permits: usize,
}

impl ConnLimiter {
    pub fn new(permits: usize) -> Self {
        ConnLimiter {
            inner: dashmap::DashMap::new(),
            permits: permits.max(1),
        }
    }

    pub async fn acquire(
        &self,
        ip: std::net::IpAddr,
    ) -> tokio::sync::OwnedSemaphorePermit {
        let sem = self
            .inner
            .entry(ip)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(self.permits)))
            .clone();
        sem.acquire_owned().await.expect("conn limiter closed")
    }
}

pub async fn run_pipeline(
    mut rx: mpsc::Receiver<Infohash>,
    mut fresh_rx: mpsc::Receiver<Infohash>,
    mut announce_rx: mpsc::Receiver<(Infohash, SocketAddr)>,
    node_routers: Arc<Vec<Arc<Router>>>,
    utp: Option<Arc<UtpSocketUdp>>,
    metrics: Arc<Metrics>,
    batch_writer: Arc<BatchWriter>,
    peer_cache: Arc<PeerCache>,
    announce_peer_cache: Arc<AnnouncePeerCache>,
    peer_outcomes: Arc<crate::storage::peer_outcomes::PeerOutcomeWriter>,
    conn_limiter: Arc<ConnLimiter>,
    config: VerifyConfig,
) {
    let pipeline_limit = Arc::new(Semaphore::new(config.pipeline_limit.max(1)));
    let fetch_limit = Arc::new(Semaphore::new(config.fetch_limit.max(1)));
    let next_router = AtomicUsize::new(0);
    // Fair-drain batch: after this many consecutive fresh/announce items, force
    // one retry-channel item so a backlogged verify queue is never starved.
    let fresh_batch: usize = 8;
    let mut fresh_streak: usize = 0;
    'pipeline: loop {
        let Ok(_pipeline_permit) = pipeline_limit.clone().acquire_owned().await else {
            break;
        };

let (ih, direct, came_from_scheduler) = loop {
            // 1) Announce (direct peer) has top priority.
            if let Ok((ih, addr)) = announce_rx.try_recv() { break (ih, Some(addr), false) }
            // 2) Fresh discoveries, batched so they cannot monopolize the queue.
            if fresh_streak < fresh_batch
                && let Ok(ih) = fresh_rx.try_recv() {
                    fresh_streak += 1;
                    break (ih, None, false);
                }
            // 3) Retry channel: guaranteed slot after every fresh_batch.
            if let Ok(ih) = rx.try_recv() {
                fresh_streak = 0;
                break (ih, None, true);
            }
            // 4) Everything momentarily empty: block on the first producer.
            break tokio::select! {
                biased;
                item = announce_rx.recv() => {
                    match item {
                        Some((ih, addr)) => (ih, Some(addr), false),
                        None => match fresh_rx.recv().await {
                            Some(ih) => {
                                fresh_streak = 0;
                                (ih, None, false)
                            }
                            None => match rx.recv().await {
                                Some(ih) => {
                                    fresh_streak = 0;
                                    (ih, None, true)
                                }
                                None => break 'pipeline,
                            },
                        },
                    }
                }
                item = fresh_rx.recv() => {
                    match item {
                        Some(ih) => {
                            fresh_streak = 0;
                            (ih, None, false)
                        }
                        None => match announce_rx.recv().await {
                            Some((ih, addr)) => (ih, Some(addr), false),
                            None => match rx.recv().await {
                                Some(ih) => {
                                    fresh_streak = 0;
                                    (ih, None, true)
                                }
                                None => break 'pipeline,
                            },
                        },
                    }
                }
                item = rx.recv() => {
                    match item {
                        Some(ih) => {
                            fresh_streak = 0;
                            (ih, None, true)
                        }
                        None => match announce_rx.recv().await {
                            Some((ih, addr)) => (ih, Some(addr), false),
                            None => match fresh_rx.recv().await {
                                Some(ih) => {
                                    fresh_streak = 0;
                                    (ih, None, false)
                                }
                                None => break 'pipeline,
                            },
                        },
                    }
                }
            };
        };

        // Store the announcing peer for this infohash so verify_infohash can use it.
        if let Some(addr) = direct {
            announce_peer_cache.insert(ih, addr);
        }

        let router = node_routers[next_router.fetch_add(1, Ordering::Relaxed) % node_routers.len()].clone();
        let is_direct = direct.is_some();
        let utp = utp.clone();
        let metrics = metrics.clone();
        let batch_writer = batch_writer.clone();
        let peer_cache = peer_cache.clone();
        let announce_peer_cache = announce_peer_cache.clone();
        let peer_outcomes = peer_outcomes.clone();
        let conn_limiter = conn_limiter.clone();
        let params = config.params.clone();
        let fetch_limit = fetch_limit.clone();
        tokio::spawn(async move {
            let _pipeline_permit = _pipeline_permit;
            if is_direct {
                metrics.announce_attempts.add(1);
            }
            metrics.verify_attempts.add(1);
            match verify_infohash(router, utp, ih, &params, metrics.clone(), peer_cache, announce_peer_cache, direct, peer_outcomes, conn_limiter, fetch_limit).await {
                VerifyResult::Success(meta) if check(&ih, &meta) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", stream = "verify", result = "pass");
                    metrics.verify_success.add(1);
                    if is_direct {
                        metrics.announce_success.add(1);
                    }
                    batch_writer.push_torrent(ih, &meta);
                    crate::trace_lifecycle!(&ih, "persist_torrents", stream = "verify", status = "ok");
                    // Only scheduler-sourced infohashes have a verification_jobs
                    // row worth deleting; fresh successes never wrote one, so
                    // skip the no-op DELETE.
                    if came_from_scheduler {
                        batch_writer.push_verified(ih);
                    }
                }
                VerifyResult::Success(_) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", stream = "verify", result = "fail");
                    metrics.sha1_mismatch.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "sha1_mismatch");
                }
                VerifyResult::NoPeers => {
                    crate::trace_lifecycle!(&ih, "verify_fail", stream = "verify", result = "no_peers");
                    metrics.source_no_peers.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_peers");
                }
                VerifyResult::SourceTimeout => {
                    crate::trace_lifecycle!(&ih, "verify_fail", stream = "verify", result = "source_timeout");
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "source_timeout");
                }
                VerifyResult::MetadataFailed => {
                    crate::trace_lifecycle!(&ih, "verify_fail", stream = "verify", result = "no_metadata");
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_metadata");
                }
            }
        });
    }
}
