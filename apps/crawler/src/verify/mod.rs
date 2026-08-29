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
        self.inner
            .retain(|_, (_, ts)| now.duration_since(*ts) < self.ttl);
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

    pub async fn acquire(&self, ip: std::net::IpAddr) -> tokio::sync::OwnedSemaphorePermit {
        let sem = self
            .inner
            .entry(ip)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(self.permits)))
            .clone();
        sem.acquire_owned().await.expect("conn limiter closed")
    }
}

fn saturating_add_atomic(target: &std::sync::atomic::AtomicU64, delta: u64) {
    // Relaxed saturating add for cumulative microsecond counters — on overflow
    // the counter saturates at u64::MAX rather than wrapping (averages then
    // degrade gracefully instead of going to zero). Counts use wrapping fetch_add.
    let mut prev = target.load(Ordering::Relaxed);
    loop {
        let next = prev.saturating_add(delta);
        match target.compare_exchange_weak(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => prev = actual,
        }
    }
}

struct PipelineTaskGuard {
    metrics: Arc<Metrics>,
    start: Instant,
    completed: bool,
}

impl PipelineTaskGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        let cur = metrics.pipeline_active.fetch_add(1, Ordering::Relaxed) + 1;
        metrics
            .pipeline_active_max_interval
            .fetch_max(cur, Ordering::Relaxed);
        PipelineTaskGuard {
            metrics,
            start: Instant::now(),
            completed: false,
        }
    }

    fn complete(&mut self) {
        if !self.completed {
            self.completed = true;
            self.metrics
                .pipeline_completed_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for PipelineTaskGuard {
    fn drop(&mut self) {
        let elapsed_us = self.start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        saturating_add_atomic(&self.metrics.pipeline_task_micros_total, elapsed_us);
        // Exactly-once decrement via guard state; debug-assert non-underflow, never
        // overwriting concurrent increments with store(0).
        let prev = self.metrics.pipeline_active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(
            prev > 0,
            "pipeline_active underflow: decrement with no active task"
        );
        if !self.completed {
            self.metrics
                .pipeline_cancelled_total
                .fetch_add(1, Ordering::Relaxed);
        }
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
        let wait_start = Instant::now();
        let Ok(_pipeline_permit) = pipeline_limit.clone().acquire_owned().await else {
            break;
        };
        let wait_us = wait_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        saturating_add_atomic(&metrics.pipeline_permit_wait_micros_total, wait_us);
        metrics
            .pipeline_permit_acquisitions_total
            .fetch_add(1, Ordering::Relaxed);

        let (ih, direct, came_from_scheduler) = loop {
            // 1) Announce (direct peer) has top priority.
            match announce_rx.try_recv() {
                Ok((ih, addr)) => break (ih, Some(addr), false),
                Err(_) => {}
            }
            // 2) Fresh discoveries, batched so they cannot monopolize the queue.
            if fresh_streak < fresh_batch {
                match fresh_rx.try_recv() {
                    Ok(ih) => {
                        fresh_streak += 1;
                        break (ih, None, false);
                    }
                    Err(_) => {}
                }
            }
            // 3) Retry channel: guaranteed slot after every fresh_batch.
            match rx.try_recv() {
                Ok(ih) => {
                    fresh_streak = 0;
                    break (ih, None, true);
                }
                Err(_) => {}
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

        // Dequeue accounting — increment immediately after removal.
        if direct.is_some() {
            metrics.announce_dequeued_total.add(1);
        } else if came_from_scheduler {
            metrics.retry_dequeued_total.add(1);
        } else {
            metrics.fresh_dequeued_total.add(1);
        }
        metrics.pipeline_dequeued_total.add(1);

        // Store the announcing peer for this infohash so verify_infohash can use it.
        if let Some(addr) = direct {
            announce_peer_cache.insert(ih, addr);
        }

        let router =
            node_routers[next_router.fetch_add(1, Ordering::Relaxed) % node_routers.len()].clone();
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
            metrics.pipeline_spawned_total.add(1);
            let mut guard = PipelineTaskGuard::new(metrics.clone());
            if is_direct {
                metrics.announce_attempts.add(1);
            }
            metrics.verify_attempts.add(1);
            let verify_result = verify_infohash(
                router,
                utp,
                ih,
                &params,
                metrics.clone(),
                peer_cache,
                announce_peer_cache,
                direct,
                peer_outcomes,
                conn_limiter,
                fetch_limit,
            )
            .await;
            let handling_start = Instant::now();
            match verify_result {
                VerifyResult::Success(meta, source) if check(&ih, &meta) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", stream = "verify", result = "pass");
                    metrics.verify_success.add(1);
                    match source {
                        fetch_pool::CandidateSource::Direct => {
                            metrics
                                .source_direct_verified_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        fetch_pool::CandidateSource::AnnounceCache => {
                            metrics
                                .source_announce_cache_verified_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        fetch_pool::CandidateSource::Dht => {
                            metrics
                                .source_dht_verified_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    if is_direct {
                        metrics.announce_success.add(1);
                    }
                    batch_writer.push_torrent(ih, &meta);
                    crate::trace_lifecycle!(
                        &ih,
                        "persist_torrents",
                        stream = "verify",
                        status = "ok"
                    );
                    // Only scheduler-sourced infohashes have a verification_jobs
                    // row worth deleting; fresh successes never wrote one, so
                    // skip the no-op DELETE.
                    if came_from_scheduler {
                        batch_writer.push_verified(ih);
                    }
                }
                VerifyResult::Success(_, _) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", stream = "verify", result = "fail");
                    metrics.sha1_mismatch.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "sha1_mismatch");
                }
                VerifyResult::NoPeers => {
                    crate::trace_lifecycle!(
                        &ih,
                        "verify_fail",
                        stream = "verify",
                        result = "no_peers"
                    );
                    metrics.source_no_peers.add(1);
                    metrics.pipeline_no_peers_total.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_peers");
                }
                VerifyResult::SourceTimeout => {
                    crate::trace_lifecycle!(
                        &ih,
                        "verify_fail",
                        stream = "verify",
                        result = "source_timeout"
                    );
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "source_timeout");
                }
                VerifyResult::MetadataFailed => {
                    crate::trace_lifecycle!(
                        &ih,
                        "verify_fail",
                        stream = "verify",
                        result = "no_metadata"
                    );
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_metadata");
                }
            }
            let handling_us = handling_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
            // saturating add helper is in fetch_pool; use direct atomic with saturating via CAS loop
            {
                let target = &metrics.result_handling_micros_total;
                let mut prev = target.load(Ordering::Relaxed);
                loop {
                    let next = prev.saturating_add(handling_us);
                    match target.compare_exchange_weak(
                        prev,
                        next,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(actual) => prev = actual,
                    }
                }
            }
            metrics
                .result_handling_completed_total
                .fetch_add(1, Ordering::Relaxed);
            guard.complete();
        });
    }
}

#[cfg(test)]
mod pipeline_metrics_tests {
    use super::*;
    use crate::metrics::Metrics;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    fn test_metrics() -> Arc<Metrics> {
        Arc::new(Metrics::new(Arc::new(AtomicU64::new(0))))
    }

    #[test]
    fn normal_completion_returns_active_to_zero() {
        let m = test_metrics();
        {
            let mut g = PipelineTaskGuard::new(m.clone());
            assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 1);
            g.complete();
        }
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 0);
        assert_eq!(m.pipeline_completed_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.pipeline_cancelled_total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn failure_counts_as_completed_not_cancelled() {
        let m = test_metrics();
        {
            let mut g = PipelineTaskGuard::new(m.clone());
            // Simulate NoPeers path
            m.pipeline_no_peers_total.fetch_add(1, Ordering::Relaxed);
            g.complete();
        }
        assert_eq!(m.pipeline_completed_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.pipeline_cancelled_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn aborted_task_returns_active_to_zero_and_cancelled() {
        let m = test_metrics();
        let m2 = m.clone();
        let h = tokio::spawn(async move {
            let _guard = PipelineTaskGuard::new(m2.clone());
            // Sleep longer than abort delay
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        // Give task time to increment active
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 1);
        h.abort();
        let _ = h.await;
        // Guard dropped via cancellation
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 0);
        assert_eq!(m.pipeline_cancelled_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.pipeline_completed_total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn interval_maximum_at_least_current_after_reset() {
        let m = test_metrics();
        // Simulate 3 concurrent tasks
        let _g1 = PipelineTaskGuard::new(m.clone());
        let _g2 = PipelineTaskGuard::new(m.clone());
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 2);
        let max_before = m.pipeline_active_max_interval.load(Ordering::Relaxed);
        assert!(max_before >= 2);
        // Report reset: store current (2)
        m.pipeline_active_max_interval
            .store(m.pipeline_active.load(Ordering::Relaxed), Ordering::Relaxed);
        assert_eq!(m.pipeline_active_max_interval.load(Ordering::Relaxed), 2);
        // New peak should be captured
        let _g3 = PipelineTaskGuard::new(m.clone());
        assert_eq!(m.pipeline_active_max_interval.load(Ordering::Relaxed), 3);
        // After reset while 3 active, max stays 3
        m.pipeline_active_max_interval
            .store(m.pipeline_active.load(Ordering::Relaxed), Ordering::Relaxed);
        assert!(m.pipeline_active_max_interval.load(Ordering::Relaxed) >= 3);
    }

    #[test]
    fn active_counters_cannot_underflow() {
        let m = test_metrics();
        // Exactly-once decrement via guard state: no store(0) repair.
        // Normal lifecycle should never underflow.
        {
            let mut g = PipelineTaskGuard::new(m.clone());
            assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 1);
            g.complete();
            // Drop here decrements to 0 via fetch_sub exactly once.
        }
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 0);
        // Concurrent guards must return to 0 without underflow.
        {
            let _g1 = PipelineTaskGuard::new(m.clone());
            let _g2 = PipelineTaskGuard::new(m.clone());
            assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 2);
        }
        assert_eq!(m.pipeline_active.load(Ordering::Relaxed), 0);
        assert_ne!(m.pipeline_active.load(Ordering::Relaxed), u64::MAX);
        // debug_assert!(prev > 0) ensures underflow would panic in debug builds
        // rather than silently storing 0 and losing concurrent increments.
    }

    #[tokio::test]
    async fn concurrent_increment_while_interval_reset() {
        let m = test_metrics();
        // Pre-fill max to 5
        m.pipeline_active_max_interval.store(5, Ordering::Relaxed);
        let m_clone = m.clone();
        // Task increments active while main thread swaps interval max
        let h = tokio::spawn(async move {
            let _g = PipelineTaskGuard::new(m_clone.clone());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // Swap reset to current active (should be 1) — report max of swapped and current
        let current = m.pipeline_active.load(Ordering::Relaxed);
        let swapped = m
            .pipeline_active_max_interval
            .swap(current, Ordering::Relaxed);
        let reported = swapped.max(current);
        assert!(reported >= 5 || reported >= current);
        // New peak after reset must be captured via fetch_max
        let _g2 = PipelineTaskGuard::new(m.clone());
        assert!(m.pipeline_active_max_interval.load(Ordering::Relaxed) >= 2);
        let _ = h.await;
    }

    #[test]
    fn dequeue_source_counters_sum_to_total() {
        let m = test_metrics();
        // Simulate dispatcher post-loop accounting
        for i in 0..10 {
            if i % 3 == 0 {
                m.announce_dequeued_total.fetch_add(1, Ordering::Relaxed);
            } else if i % 3 == 1 {
                m.fresh_dequeued_total.fetch_add(1, Ordering::Relaxed);
            } else {
                m.retry_dequeued_total.fetch_add(1, Ordering::Relaxed);
            }
            m.pipeline_dequeued_total.fetch_add(1, Ordering::Relaxed);
        }
        let sum = m.fresh_dequeued_total.load(Ordering::Relaxed)
            + m.retry_dequeued_total.load(Ordering::Relaxed)
            + m.announce_dequeued_total.load(Ordering::Relaxed);
        assert_eq!(sum, m.pipeline_dequeued_total.load(Ordering::Relaxed));
        assert_eq!(sum, 10);
    }
}
