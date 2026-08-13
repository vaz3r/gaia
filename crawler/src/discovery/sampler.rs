use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gaia_core::Id20;
use gaia_dht::DhtHandle;
use rand::seq::SliceRandom;
use rand::thread_rng;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::stats::CrawlStats;
use crate::discovery::FetchRequest;
use crate::storage::{ScannedStatus, Storage};

/// How long a node that failed to answer is retried. 30s: a non-responsive
/// node may be offline/overloaded, so back off harder than a healthy 0-new
/// node (which is re-queried after STALE_BACKOFF). Still short enough to
/// recover from transient UDP loss.
const FAIL_BACKOFF: Duration = Duration::from_secs(30);
/// How long a node that answered but returned zero *new* hashes is skipped on
/// its FIRST empty response. A healthy node may legitimately have nothing new
/// once; re-query it soon since it may pick up hashes shortly. Only after
/// `STALE_GRADUATION` consecutive empties does it earn the longer shelf.
const STALE_BACKOFF: Duration = Duration::from_secs(60);
/// After this many consecutive 0-new responses, a node graduates to the long
/// backoff (it has stopped yielding). Tolerates normal variance — a single
/// unlucky empty response must not shelve a productive node for 5 minutes.
const STALE_GRADUATION: u32 = 3;
/// Long backoff applied once a node has repeatedly returned nothing new.
const STALE_LONG_BACKOFF: Duration = Duration::from_secs(300);
/// Cap on the per-node interval map (LRU-evicted).
const INTERVAL_MAP_CAP: usize = 8192;
/// Cap on the per-node quality map (LRU-evicted).
const NODE_STATS_CAP: usize = 32_768;
/// Minimum time to wait when no node is re-queryable.
const MIN_LOOP_DELAY: Duration = Duration::from_millis(10);
/// Time to wait for the routing table to populate before warning.
const BOOTSTRAP_WAIT: Duration = Duration::from_secs(15);
/// Number of random ready nodes to sample when picking a target; the highest
/// quality among them wins, spreading queries across the routing table. A
/// larger sample keeps the sampler from converging on a few productive nodes,
/// reaching more distinct BEP 51 nodes and surfacing more unique hashes.
const PICK_CANDIDATES: usize = 256;
/// Safety cap on a single `sample_infohashes` round-trip. The DHT actor
/// resolves a query via a oneshot reply; if a peer answers with a KRPC error
/// the actor can drop that reply without firing it (an irontide quirk), which
/// would otherwise hang the loop forever. This timeout bounds the wait and the
/// node is retried later via `FAIL_BACKOFF`.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Sampler loop configuration.
#[derive(Debug, Clone)]
pub struct SamplerConfig {
    /// Maximum aggregate `sample_infohashes` queries issued per second across
    /// all sampling loops (0 = unlimited).
    pub queries_per_second: usize,
    /// Number of concurrent sampling loops sharing the query budget.
    pub concurrency: usize,
    /// An infohash is emitted only after this many distinct sampling
    /// responses reported it. Higher values cull the long-tail of junk but
    /// delay rare-but-valid releases.
    pub min_seen: u32,
    /// Optional shadow threshold: observe what `--min-seen` would filter while
    /// keeping the live threshold. Entry lifetime = max(min_seen, shadow).
    pub min_seen_shadow: Option<u32>,
    /// Upper bound (seconds) on the per-node re-query interval advertised by
    /// BEP 51 nodes. Nodes that report longer intervals are still re-queried
    /// after this period so the routing table keeps growing.
    pub max_interval_secs: u64,
}

/// Result of routing one sampled hash through `emit_sample`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmitOutcome {
    /// The pipeline is shut down; the sampler loop must stop.
    Shutdown,
    /// The hash was already known to this loop (a repeat).
    Repeat,
    /// The hash was newly seen by this loop (first sighting).
    New,
}

/// A node address → (last_query_time, re-query interval) map with LRU cap.
struct IntervalMap {
    map: HashMap<SocketAddr, (Instant, Duration)>,
    order: VecDeque<SocketAddr>,
    cap: usize,
}

impl IntervalMap {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    /// True if `addr` may be queried now.
    fn is_ready(&self, addr: &SocketAddr, now: Instant) -> bool {
        match self.map.get(addr) {
            Some((last, interval)) => now.saturating_duration_since(*last) >= *interval,
            None => true,
        }
    }

    /// Record a query to `addr` with its returned `interval`.
    fn record(&mut self, addr: SocketAddr, interval: Duration, now: Instant) {
        if let Some(entry) = self.map.get_mut(&addr) {
            *entry = (now, interval);
            return;
        }
        self.map.insert(addr, (now, interval));
        self.order.push_back(addr);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Per-node sampling quality for biasing target selection toward productive
/// (BEP 51 capable) nodes.
#[derive(Default)]
struct NodeStat {
    samples: u64,
    failures: u64,
    /// Consecutive 0-new-hash responses. Resets on a response with new hashes
    /// or on a successful sample; drives graduation to the long stale backoff.
    consecutive_stale: u32,
}

impl NodeStat {
    /// Higher is better; failures weigh double so flaky nodes sink.
    fn score(&self) -> i64 {
        self.samples as i64 - 2 * self.failures as i64
    }
}

/// Bounded, FIFO-evicted node quality map.
struct NodeStats {
    map: HashMap<SocketAddr, NodeStat>,
    order: VecDeque<SocketAddr>,
    cap: usize,
}

impl NodeStats {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn ensure_locked(&mut self, addr: SocketAddr) {
        if !self.map.contains_key(&addr) {
            self.order.push_back(addr);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
            self.map.insert(addr, NodeStat::default());
        }
    }

    fn score_locked(&self, addr: &SocketAddr) -> i64 {
        self.map.get(addr).map_or(0, |s| s.score())
    }

    /// Register a query round against `addr`. `total_samples` is how many
    /// infohashes the response reported (0 = empty/stale response). Returns the
    /// updated consecutive-stale count so the caller can graduate the backoff.
    fn record_result_locked(&mut self, addr: SocketAddr, total_samples: usize) -> u32 {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.samples = stat.samples.saturating_add(total_samples as u64);
        if total_samples == 0 {
            stat.consecutive_stale = stat.consecutive_stale.saturating_add(1);
            stat.failures = stat.failures.saturating_add(1);
        } else {
            stat.consecutive_stale = 0;
        }
        stat.consecutive_stale
    }

    fn record_failure_locked(&mut self, addr: SocketAddr) {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.failures = stat.failures.saturating_add(1);
        stat.consecutive_stale = 0;
    }

    fn record_hang_locked(&mut self, addr: SocketAddr) {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.failures = stat.failures.saturating_add(1);
        stat.consecutive_stale = 0;
    }
}

/// A simple rate limiter that sleeps `1/qps` between queries.
struct QpsGate {
    per_query: Duration,
}

impl QpsGate {
    fn new(qps: usize) -> Self {
        let per_query = if qps == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / qps as f64)
        };
        Self { per_query }
    }

    async fn acquire(&self) {
        if !self.per_query.is_zero() {
            tokio::time::sleep(self.per_query).await;
        }
    }
}

/// BEP 51 keyspace traversal: sample infohashes across random targets while
/// honoring each node's interval, emitting unique infohashes (with their
/// popularity) into the pipeline. Spawns `concurrency` independent loops that
/// share one query budget.
pub struct Sampler {
    handle: DhtHandle,
    emit: mpsc::Sender<crate::discovery::FetchRequest>,
    storage: Storage,
    stats: Arc<CrawlStats>,
    cfg: SamplerConfig,
    shutdown: CancellationToken,
    shared: crate::redis::SharedState,
    seen: crate::bloom::SharedBloom,
    liveness: Arc<crate::discovery::LivenessCounter>,
}

impl Sampler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: DhtHandle,
        emit: mpsc::Sender<crate::discovery::FetchRequest>,
        storage: Storage,
        cfg: &SamplerConfig,
        stats: Arc<CrawlStats>,
        shutdown: CancellationToken,
        shared: crate::redis::SharedState,
        seen: crate::bloom::SharedBloom,
        liveness: Arc<crate::discovery::LivenessCounter>,
    ) -> Self {
        Self {
            handle,
            emit,
            storage,
            stats,
            cfg: cfg.clone(),
            shutdown,
            shared,
            seen,
            liveness,
        }
    }

    /// Run all sampling loops until the emit channel closes (shutdown).
    pub async fn run(&self) {
        self.wait_for_bootstrap().await;

        let max_interval = Duration::from_secs(self.cfg.max_interval_secs.max(1));
        let gate = Arc::new(QpsGate::new(self.cfg.queries_per_second));
        // Node backoff/quality is a property of the node, not the loop. A
        // single shared copy keeps the footprint O(table) instead of O(table ×
        // loops): 64 loops each duplicating a per-node map caused ~130 MB of
        // steady RSS growth as the routing table churned through distinct
        // addrs toward the per-loop caps (8192/32768).
        let intervals = Arc::new(std::sync::Mutex::new(IntervalMap::new(INTERVAL_MAP_CAP)));
        let node_stats = Arc::new(std::sync::Mutex::new(NodeStats::new(NODE_STATS_CAP)));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..self.cfg.concurrency.max(1) {
            let mut loop_ = SamplerLoop {
                handle: self.handle.clone(),
                emit: self.emit.clone(),
                storage: self.storage.clone(),
                stats: self.stats.clone(),
                intervals: intervals.clone(),
                gate: gate.clone(),
                node_stats: node_stats.clone(),
                min_seen: self.cfg.min_seen.max(1),
                min_seen_shadow: self.cfg.min_seen_shadow,
                max_interval,
                cursor: 0,
                shared: self.shared.clone(),
                seen_bloom: self.seen.clone(),
                liveness: self.liveness.clone(),
                shutdown: self.shutdown.clone(),
            };
            tasks.spawn(async move { loop_.run_loop().await });
        }
        while tasks.join_next().await.is_some() {}
    }

    /// Wait until the routing table has nodes, warning on timeout.
    async fn wait_for_bootstrap(&self) {
        let start = Instant::now();
        while start.elapsed() < BOOTSTRAP_WAIT {
            if let Ok(n) = self.handle.node_count().await {
                if n > 0 {
                    debug!(nodes = n, "routing table populated");
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        warn!("bootstrap did not populate routing table within {BOOTSTRAP_WAIT:?}; continuing");
    }
}

/// One independent sampling loop with its own rotating cursor. Per-node state
/// (intervals, quality) is SHARED across loops via `Arc<Mutex<...>>`.
struct SamplerLoop {
    handle: DhtHandle,
    emit: mpsc::Sender<crate::discovery::FetchRequest>,
    storage: Storage,
    stats: Arc<CrawlStats>,
    intervals: Arc<std::sync::Mutex<IntervalMap>>,
    gate: Arc<QpsGate>,
    node_stats: Arc<std::sync::Mutex<NodeStats>>,
    min_seen: u32,
    min_seen_shadow: Option<u32>,
    max_interval: Duration,
    shared: crate::redis::SharedState,
    seen_bloom: crate::bloom::SharedBloom,
    liveness: Arc<crate::discovery::LivenessCounter>,
    /// Rotating cursor over the routing table: each pick advances past the
    /// previous node so loops cycle through the whole table instead of
    /// re-selecting the same high-score nodes (the "no ready node" starvation).
    cursor: usize,
    shutdown: CancellationToken,
}

impl SamplerLoop {
    async fn run_loop(&mut self) {
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            let nodes = self.handle.get_routing_nodes().await;
            if nodes.is_empty() {
                debug!("routing table empty, waiting");
                tokio::time::sleep(MIN_LOOP_DELAY).await;
                continue;
            }

            let now = Instant::now();
            let Some((target, node_addr)) =
                pick_target(&mut self.cursor, &self.intervals, &self.node_stats, &nodes, now)
            else {
                let ready = {
                    let iv = self.intervals.lock().unwrap();
                    nodes.iter().filter(|(_, a)| iv.is_ready(a, now)).count()
                };
                debug!(
                    ready,
                    total = nodes.len(),
                    "pick_target found no ready node"
                );
                tokio::time::sleep(MIN_LOOP_DELAY).await;
                continue;
            };

            self.gate.acquire().await;
            let result = tokio::time::timeout(SAMPLE_TIMEOUT, self.handle.sample_infohashes(target)).await;
            let result = match result {
                Ok(r) => r,
                Err(_elapsed) => {
                    debug!(%node_addr, "sample_infohashes hung, timed out");
                    // A non-responsive node gets a longer backoff than a healthy
                    // 0-new node — it may be offline. Reset the stale counter.
                    self.intervals.lock().unwrap().record(node_addr, FAIL_BACKOFF, now);
                    self.node_stats.lock().unwrap().record_hang_locked(node_addr);
                    continue;
                }
            };
            match result {
                Ok(res) => {
                    let advertised = Duration::from_secs(res.interval.max(0) as u64);
                    let mut interval = advertised.min(self.max_interval);
                    let total_samples = res.samples.len();
                    debug!(
                        %node_addr,
                        interval_secs = res.interval,
                        capped = interval.as_secs(),
                        samples = total_samples,
                        closer_nodes = res.nodes.len(),
                        "sample_infohashes ok"
                    );
                    let mut new_count = 0u32;
                    for sample in res.samples {
                        // `target` is the sampled node's own ID — the source of
                        // this report for the liveness counter.
                        match self.emit_sample(sample, target).await {
                            EmitOutcome::Shutdown => return,
                            EmitOutcome::Repeat => {}
                            EmitOutcome::New => new_count += 1,
                        }
                    }
                    // A node that echoed only already-known hashes is stale. On
                    // its FIRST empty response use the short backoff (it may
                    // pick up hashes soon); only after STALE_GRADUATION
                    // consecutive empties does it earn the long shelf. A
                    // response with new hashes resets the counter.
                    let stale_count = self
                        .node_stats
                        .lock()
                        .unwrap()
                        .record_result_locked(node_addr, total_samples);
                    if new_count == 0 {
                        if stale_count >= STALE_GRADUATION {
                            interval = interval.max(STALE_LONG_BACKOFF);
                        } else {
                            interval = interval.max(STALE_BACKOFF);
                        }
                    }
                    self.intervals.lock().unwrap().record(node_addr, interval, now);
                    // Response nodes were already fed back into the routing table
                    // by the DHT actor.
                }
                Err(e) => {
                    debug!(error = %e, %node_addr, "sample_infohashes failed");
                    // A query error (timeout/refused) gets the longer
                    // non-response backoff; reset the stale counter.
                    self.intervals
                        .lock()
                        .unwrap()
                        .record(node_addr, FAIL_BACKOFF, now);
                    self.node_stats.lock().unwrap().record_failure_locked(node_addr);
                }
            }
        }
    }

    /// Route a sampled hash through the shared liveness gate, the bloom/DB
    /// pre-filter, and shared fleet dedup. `source` is the DHT node ID that
    /// reported `hash`. Reports whether the hash was new to the liveness
    /// counter. `EmitOutcome::Shutdown` means the pipeline is shut down.
    async fn emit_sample(&mut self, hash: Id20, source: Id20) -> EmitOutcome {
        self.stats
            .hashes_sampled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let now = Instant::now();
        // Record the report BEFORE the bloom/DB gate so shadow mode can observe
        // a hash accumulating distinct sources even after a live emit (the
        // entry survives to the shadow threshold). In-process DashMap op, not
        // a Redis round-trip.
        let outcome = self.liveness.record(hash.as_bytes(), source, now);
        let new = match outcome {
            crate::discovery::RecordOutcome::New => true,
            crate::discovery::RecordOutcome::Repeat => false,
            crate::discovery::RecordOutcome::Gained { distinct } => distinct == 1,
        };

        // Shadow accounting: if the hash reached the shadow threshold, count it
        // as "would be emitted" and remove the entry (its purpose is served).
        if let Some(shadow) = self.min_seen_shadow {
            if self.liveness.live_count(hash.as_bytes(), now) as u32 >= shadow {
                self.stats
                    .shadow_emitted
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.liveness.remove(hash.as_bytes());
            }
        }

        // Never queue hashes already accepted/filtered or still in backoff.
        // The bloom caches *terminal* skip verdicts (Ok/Skipped) so repeated
        // re-sampling of dead hashes stops hitting the database after the first
        // authoritative check per hash. Hashes in a *backoff* window are still
        // skipped but NOT cached, so they can be retried once the backoff
        // expires — matching the pre-bloom behavior.
        if self.seen_bloom.contains(hash.as_bytes()) {
            return if new { EmitOutcome::New } else { EmitOutcome::Repeat };
        }
        match self.storage.scan_status(hash.as_bytes()) {
            Ok(Some(ScannedStatus::Ok | ScannedStatus::Skipped)) => {
                self.seen_bloom.insert(hash.as_bytes());
                return if new { EmitOutcome::New } else { EmitOutcome::Repeat };
            }
            Ok(Some(ScannedStatus::Failed { next_attempt, .. })) if next_attempt > unix_secs() => {
                return if new { EmitOutcome::New } else { EmitOutcome::Repeat };
            }
            _ => {}
        }

        // Liveness gate: only emit once enough distinct sources corroborated
        // this hash within the window.
        let distinct = self.liveness.live_count(hash.as_bytes(), now) as u32;
        if distinct < self.min_seen {
            return if new { EmitOutcome::New } else { EmitOutcome::Repeat };
        }

        // Fleet-wide dedup: if another instance already emitted this hash,
        // skip it here (best-effort; Redis failure returns false).
        if self.shared.seen_contains(hash.as_bytes()).await {
            return if new { EmitOutcome::New } else { EmitOutcome::Repeat };
        }

        self.stats
            .hashes_unique
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ok = self
            .emit
            .send(FetchRequest {
                hash,
                occurrences: distinct,
                peer_hint: None,
            })
            .await
            .is_ok();
        self.shared.seen_add(hash.as_bytes()).await;

        // Entry lifetime is governed by max(min_seen, min_seen_shadow): a live
        // emit must not delete an entry shadow mode still needs to observe.
        let shadow = self.min_seen_shadow.unwrap_or(0);
        if shadow <= self.min_seen {
            self.liveness.remove(hash.as_bytes());
        }

        if ok { EmitOutcome::New } else { EmitOutcome::Shutdown }
    }
}

/// Pick a ready node and query it with a target equal to its own node ID. The
/// DHT actor resolves `sample_infohashes` to `closest(target, 1)`, so a target
/// equal to the node's own ID makes the actor query exactly this node.
///
/// A per-loop rotating cursor starts the scan at a different position each
/// call, so consecutive picks advance through the ready list instead of
/// re-selecting the same high-score nodes (the "no ready node" starvation when
/// few nodes are marked ready). Within the window starting at the cursor, the
/// highest-quality node wins. A few cooling nodes cannot starve the sampler
/// because every ready node is a candidate.
fn pick_target(
    cursor: &mut usize,
    intervals: &Arc<std::sync::Mutex<IntervalMap>>,
    node_stats: &Arc<std::sync::Mutex<NodeStats>>,
    nodes: &[(Id20, SocketAddr)],
    now: Instant,
) -> Option<(Id20, SocketAddr)> {
    let mut rng = thread_rng();
    let iv = intervals.lock().unwrap();
    let ns = node_stats.lock().unwrap();
    let mut ready: Vec<(Id20, SocketAddr)> = nodes
        .iter()
        .filter(|(_, a)| iv.is_ready(a, now))
        .map(|(id, addr)| (*id, *addr))
        .collect();
    if ready.is_empty() {
        return None;
    }
    // Rotate: advance the cursor past the previously-picked node so we don't
    // keep landing on the same spot of the ready list.
    *cursor = cursor.checked_add(1).unwrap_or(0) % ready.len().max(1);
    let rot = *cursor % ready.len();
    // Rotate the ready list so the scan window starts at the cursor, then
    // shuffle only within a bounded window to keep coverage broad.
    ready.rotate_left(rot);
    ready.shuffle(&mut rng);
    let sample = ready.iter().take(PICK_CANDIDATES);
    let mut best: Option<(i64, Id20, SocketAddr)> = None;
    for (id, addr) in sample {
        let score = ns.score_locked(addr);
        if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
            best = Some((score, *id, *addr));
        }
    }
    // Target = the node's own ID so the actor queries this exact node.
    best.map(|(_, id, addr)| (id, addr))
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Id20 {
        Id20([n; 20])
    }

    #[test]
    fn interval_map_honors_backoff() {
        let mut m = IntervalMap::new(10);
        let now = Instant::now();
        let addr: SocketAddr = "127.0.0.1:6881".parse().unwrap();

        assert!(m.is_ready(&addr, now));
        m.record(addr, Duration::from_secs(60), now);
        assert!(!m.is_ready(&addr, now + Duration::from_secs(1)));
        assert!(
            m.is_ready(&addr, now + Duration::from_secs(61)),
            "queryable after interval elapses"
        );
    }

    #[test]
    fn interval_map_zero_interval_always_ready() {
        let mut m = IntervalMap::new(10);
        let now = Instant::now();
        let addr: SocketAddr = "127.0.0.1:6882".parse().unwrap();

        m.record(addr, Duration::ZERO, now);
        assert!(m.is_ready(&addr, now));
    }

    #[test]
    fn interval_map_evicts_oldest() {
        let mut m = IntervalMap::new(2);
        let now = Instant::now();
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let c: SocketAddr = "127.0.0.1:3".parse().unwrap();

        m.record(a, Duration::from_secs(60), now);
        m.record(b, Duration::from_secs(60), now);
        m.record(c, Duration::from_secs(60), now);
        assert!(m.map.len() <= 2, "map must stay capped");
        assert!(!m.map.contains_key(&a), "oldest entry evicted first");
    }

    #[test]
    fn qps_gate_zero_is_unlimited() {
        let g = QpsGate::new(0);
        assert!(g.per_query.is_zero());
    }

    #[test]
    fn node_stats_score_penalizes_failures() {
        let mut ns = NodeStats::new(16);
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        ns.ensure_locked(a);
        ns.ensure_locked(b);
        ns.map.get_mut(&a).unwrap().samples = 10;
        ns.map.get_mut(&b).unwrap().failures = 3;
        assert!(ns.score_locked(&a) > ns.score_locked(&b));
        assert_eq!(ns.score_locked(&a), 10);
        assert_eq!(ns.score_locked(&b), -6);
    }

    #[test]
    fn node_stats_evicts_oldest() {
        let mut ns = NodeStats::new(2);
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let c: SocketAddr = "127.0.0.1:3".parse().unwrap();
        ns.ensure_locked(a);
        ns.ensure_locked(b);
        ns.ensure_locked(c);
        assert!(ns.map.len() <= 2);
        assert!(!ns.map.contains_key(&a));
    }

    #[test]
    fn pick_target_skips_cooling_node() {
        let mut intervals = IntervalMap::new(16);
        let now = Instant::now();
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let nodes = vec![(id(1), a), (id(2), b)];

        intervals.record(a, Duration::from_secs(60), now);
        let (target, addr) = pick_target(
            &mut 0,
            &Arc::new(std::sync::Mutex::new(intervals)),
            &Arc::new(std::sync::Mutex::new(NodeStats::new(16))),
            &nodes,
            now,
        )
        .unwrap();
        assert_eq!(addr, b, "cooling node must be skipped");
        assert_eq!(target, id(2), "target must be the picked node's own ID");
    }

    #[test]
    fn pick_target_none_when_all_cooling() {
        let mut intervals = IntervalMap::new(16);
        let now = Instant::now();
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let nodes = vec![(id(1), a), (id(2), b)];
        intervals.record(a, Duration::from_secs(60), now);
        intervals.record(b, Duration::from_secs(60), now);
        let iv = Arc::new(std::sync::Mutex::new(intervals));
        let ns = Arc::new(std::sync::Mutex::new(NodeStats::new(16)));
        assert!(pick_target(&mut 0, &iv, &ns, &nodes, now).is_none());

        // After the interval elapses, a node becomes selectable again.
        let later = now + Duration::from_secs(61);
        assert!(pick_target(&mut 0, &iv, &ns, &nodes, later).is_some());
    }

    #[test]
    fn pick_target_prefers_productive_node() {
        let mut node_stats = NodeStats::new(16);
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        node_stats.ensure_locked(b);
        node_stats.ensure_locked(a);
        node_stats.map.get_mut(&b).unwrap().samples = 50;
        node_stats.map.get_mut(&a).unwrap().failures = 5;

        let intervals = IntervalMap::new(16);
        let nodes = vec![(id(1), a), (id(2), b)];
        let now = Instant::now();
        let (target, addr) = pick_target(
            &mut 0,
            &Arc::new(std::sync::Mutex::new(intervals)),
            &Arc::new(std::sync::Mutex::new(node_stats)),
            &nodes,
            now,
        )
        .unwrap();
        assert_eq!(addr, b, "productive node must be picked over penalized one");
        assert_eq!(target, id(2));
    }
}
