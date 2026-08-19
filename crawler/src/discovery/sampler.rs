use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gaia_core::Id20;
use gaia_dht::DhtHandle;
use rand::Rng;
use rand::RngCore;
use rand::thread_rng;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::stats::CrawlStats;
use crate::discovery::FetchRequest;
use crate::storage::{ScannedStatus, Storage};

/// How long a node that failed to answer is retried. 10s: a non-responsive
/// node may be offline/overloaded, so back off harder than bitmagnet's 5s
/// floor yet stay short enough to recover from transient UDP loss.
const FAIL_BACKOFF: Duration = Duration::from_secs(10);
/// Floor on the re-query interval so a node advertising interval=0 is never
/// re-queried in a tight loop (bitmagnet's 5s minimum between queries to the
/// same node).
const RESAMPLE_FLOOR: Duration = Duration::from_secs(5);
/// Freshness window for the last-responded sampling gate (bitmagnet's
/// `lastRespondedAt`). A node is a sampling candidate only if it responded within
/// this window. Matched to the empty/stale backoff (60s) so productive nodes
/// rotate back in as their backoff expires while dead/stale nodes drop out. This
/// keeps the pool full of LIVE nodes, so the follow-up direct get_peers to the
/// same node returns peer `values` instead of timing out / going empty.
const LAST_RESPONDED_WINDOW: Duration = Duration::from_secs(90);
/// A node that failed more recently than it last responded within this window is
/// deprioritized (it keeps timing out; don't burn queries on it).
const FAIL_DEPRIORITIZE_WINDOW: Duration = Duration::from_secs(300);
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
/// node is retried later via `FAIL_BACKOFF`. 6s: legitimate BEP 51 responses
/// arrive in 1-3s; the old 15s tied up sampler loops on dead nodes.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(6);
/// Refill size for the shared candidate queue feeder (bitmagnet's
/// `getNodesForSampleInfoHashes(60)` equivalent). Raised from 60 to 600/sec:
/// with 2048 loops (512/concurrency default) the old 60/sec cap starved 98% of
/// loops. Bitmagnet's channels are 10×scale deep; 600/sec keeps all loops fed
/// while the shared `sampler-qps` gate still bounds the real query rate.
const CANDIDATE_BATCH: usize = 600;
/// Cap on the shared candidate queue.
const CANDIDATE_QUEUE_CAP: usize = 8192;

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
    /// A single-source hash is emitted only after this many report events
    /// from the SAME source (the sparse/stalled discriminator). Sightings ≥ 2
    /// from one source means the node kept re-reporting the hash = live; a
    /// first sighting alone from a backoff-stalled node is dead. 0/1 disables
    /// (emit on first sighting as before).
    pub min_sightings: u32,
    /// Optional shadow threshold: observe what `--min-seen` would filter while
    /// keeping the live threshold. Entry lifetime = max(min_seen, shadow).
    pub min_seen_shadow: Option<u32>,
    /// Upper bound (seconds) on the per-node re-query interval advertised by
    /// BEP 51 nodes. Nodes that report longer intervals are still re-queryed
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
    /// True if this node has successfully responded to a `sample_infohashes`
    /// query, proving BEP 51 capability. Bitmagnet's equivalent is
    /// `bep51Support != protocolSupportNo` in `node.go`.
    bep51_capable: bool,
    /// Instant the node last responded to a `sample_infohashes` query.
    /// Bitmagnet's `lastRespondedAt`: `IsSampleInfoHashesCandidate` requires
    /// `lastRespondedAt < now-5s`, i.e. only nodes that demonstrably came back
    /// recently stay in the sampling pool. This keeps the sampled-node pool full
    /// of LIVE nodes, so the subsequent direct get_peers to the SAME node is far
    /// more likely to return peer `values` instead of timing out / going empty
    /// (the 68%-empty / 47%-timeout wall).
    last_responded: Option<Instant>,
    /// Instant the node last FAILED to respond (timeout/refused). A node with no
    /// recent success and recent failures is stale; exclude it from sampling.
    last_failed: Option<Instant>,
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

    /// True if any tracked node has a positive score (yielded samples and not
    /// bled them back out in failures). Used by `pick_target` to restrict the
    /// candidate pool to proven BEP 51 producers when any exist.
    fn has_any_proven(&self) -> bool {
        self.map.values().any(|s| s.bep51_capable)
    }

    /// True if `addr` has proven BEP 51 capability (successfully responded to
    /// a `sample_infohashes` query). Bitmagnet's equivalent is
    /// `IsSampleInfoHashesCandidate()` checking `bep51Support != protocolSupportNo`.
    fn is_bep51_capable(&self, addr: &SocketAddr) -> bool {
        self.map.get(addr).map_or(false, |s| s.bep51_capable)
    }

    /// Bitmagnet's `IsSampleInfoHashesCandidate` freshness test: a node is a
    /// valid sampling candidate only if it responded recently and has NOT failed
    /// more recently than it responded. Querying a stale node yields the same
    /// reporting-node that then times out / goes empty on direct get_peers (the
    /// 68%-empty / 47%-timeout wall); gating on freshness keeps the pool full of
    /// nodes that demonstrably answer, so the follow-up direct get_peers returns
    /// peer `values` far more often. Nodes with no recorded history are allowed
    /// in (cold-start coverage until first response).
    fn is_sampling_fresh(&self, addr: &SocketAddr, now: Instant) -> bool {
        let Some(stat) = self.map.get(addr) else {
            return true;
        };
        let responded = match stat.last_responded {
            Some(t) if now >= t => now.saturating_duration_since(t),
            Some(_) => Duration::ZERO,
            None => return true,
        };
        // Fresh if it responded within the window; a node that only failed is
        // stale unless it has no success yet.
        let fresh_window = LAST_RESPONDED_WINDOW;
        responded <= fresh_window
    }

    /// True if `addr` has been proven dead-ish (failed more recently than it
    /// responded), used to deprioritize nodes that keep timing out.
    fn is_recently_failing(&self, addr: &SocketAddr, now: Instant) -> bool {
        let Some(stat) = self.map.get(addr) else {
            return false;
        };
        match (stat.last_responded, stat.last_failed) {
            (Some(r), Some(f)) if now >= f && f > r => {
                now.saturating_duration_since(f) <= FAIL_DEPRIORITIZE_WINDOW
            }
            (None, Some(f)) => now.saturating_duration_since(f) <= FAIL_DEPRIORITIZE_WINDOW,
            _ => false,
        }
    }

    /// Register a query round against `addr`. `total_samples` is how many
    /// infohashes the response reported (0 = empty/stale response). Returns the
    /// updated consecutive-stale count so the caller can graduate the backoff.
    fn record_result_locked(&mut self, addr: SocketAddr, total_samples: usize, now: Instant) -> u32 {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.samples = stat.samples.saturating_add(total_samples as u64);
        // Any successful response proves BEP 51 capability (bitmagnet's
        // `bep51Support = protocolSupportYes`).
        stat.bep51_capable = true;
        // Bitmagnet's `lastRespondedAt`: a successful response refreshes the
        // node's freshness so it stays in the sampling pool.
        stat.last_responded = Some(now);
        if total_samples == 0 {
            stat.consecutive_stale = stat.consecutive_stale.saturating_add(1);
            stat.failures = stat.failures.saturating_add(1);
        } else {
            stat.consecutive_stale = 0;
        }
        stat.consecutive_stale
    }

    fn record_failure_locked(&mut self, addr: SocketAddr, now: Instant) {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.failures = stat.failures.saturating_add(1);
        stat.consecutive_stale = 0;
        stat.last_failed = Some(now);
    }

    fn record_hang_locked(&mut self, addr: SocketAddr, now: Instant) {
        self.ensure_locked(addr);
        let stat = self.map.get_mut(&addr).expect("ensured");
        stat.failures = stat.failures.saturating_add(1);
        stat.consecutive_stale = 0;
        stat.last_failed = Some(now);
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
    /// Shared rotating `sample_infohashes` target, owned by the caller so the
    /// grower and all sampler loops query the SAME region (bitmagnet's shared
    /// `soughtNodeID`).
    sought_rx: tokio::sync::watch::Receiver<Id20>,
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
        sought_rx: tokio::sync::watch::Receiver<Id20>,
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
            sought_rx,
        }
    }

    /// Run all sampling loops until the emit channel closes (shutdown).
    pub async fn run(&self) {
        self.wait_for_bootstrap().await;

        let max_interval = Duration::from_secs(self.cfg.max_interval_secs.max(1));
        let gate = Arc::new(QpsGate::new(self.cfg.queries_per_second));
        // Bitmagnet's shared `soughtNodeID`: one target used by ALL sampling
        // loops (and the grower's find_node), rotated on a timer by the caller.
        // BEP 51 nodes store infohashes concentrated near their OWN node ID;
        // querying each node with a DIFFERENT random target makes it return the
        // same "home region" hashes over and over (the ~95% repeat rate).
        // Aiming every query at ONE region for a short window makes each node
        // return the hashes IT stores nearest that region — a coherent,
        // non-overlapping slice — so new-hash yield per query stays high, and
        // rotating the region every few seconds spreads coverage across the
        // keyspace.
        // Node backoff/quality is a property of the node, not the loop. A
        // single shared copy keeps the footprint O(table) instead of O(table ×
        // loops): 64 loops each duplicating a per-node map caused ~130 MB of
        // steady RSS growth as the routing table churned through distinct
        // addrs toward the per-loop caps (8192/32768).
        let intervals = Arc::new(std::sync::Mutex::new(IntervalMap::new(INTERVAL_MAP_CAP)));
        let node_stats = Arc::new(std::sync::Mutex::new(NodeStats::new(NODE_STATS_CAP)));

        // Load persisted per-node state from Redis so the sampler converges on
        // productive nodes immediately instead of re-learning them.
        let now = Instant::now();
        let interval_rows = self.shared.sampler_intervals_get().await;
        let quality_rows = self.shared.sampler_quality_get().await;
        for (addr, elapsed, interval) in interval_rows {
            let mut iv = intervals.lock().unwrap();
            if let Ok(addr) = addr.parse() {
                // `elapsed` = seconds since the node was last queried (stored
                // on shutdown); rebase onto this process's clock.
                let last = now
                    .checked_sub(Duration::from_secs(elapsed.max(0) as u64))
                    .unwrap_or(now);
                iv.record(addr, Duration::from_secs(interval), last);
            }
        }
        for (addr, samples, failures, stale, bep51_capable) in quality_rows {
            let mut ns = node_stats.lock().unwrap();
            if let Ok(addr) = addr.parse() {
                ns.ensure_locked(addr);
                if let Some(s) = ns.map.get_mut(&addr) {
                    s.samples = samples;
                    s.failures = failures;
                    s.consecutive_stale = stale;
                    s.bep51_capable = bep51_capable;
                }
            }
        }

        let mut tasks = tokio::task::JoinSet::new();
        // Shared candidate queue (bitmagnet's `nodesForSampleInfoHashes`): ONE
        // background feeder refreshes the routing table once per second and
        // pushes ready candidates here; the loops pop one candidate at a time.
        // This eliminates the O(table) `get_routing_nodes()` + `pick_target`
        // full-scan-per-loop that starved the sampler at high loop counts
        // (1024 loops × 150k nodes per iteration).
        let candidates = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        {
            let handle = self.handle.clone();
            let intervals = intervals.clone();
            let node_stats = node_stats.clone();
            let shutdown = self.shutdown.clone();
            let candidates = candidates.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = shutdown.cancelled() => return,
                        _ = interval.tick() => {}
                    }
                    let nodes = handle.get_routing_nodes().await;
                    if nodes.is_empty() {
                        continue;
                    }
                    let now = Instant::now();
                    let ready: Vec<(Id20, SocketAddr)> = {
                        let iv = intervals.lock().unwrap();
                        nodes
                            .iter()
                            .filter(|(_, a)| iv.is_ready(a, now))
                            .map(|(id, addr)| (*id, *addr))
                            .collect()
                    };
                    if ready.is_empty() {
                        continue;
                    }
                    // Bitmagnet's candidate selection: prefer nodes that are both
                    // BEP51-capable AND recently-responded (fresh). Freshness is
                    // the key conversion gate — a node that just responded is
                    // alive, so the follow-up direct get_peers to it returns peer
                    // `values` instead of timing out / going empty. Recently-
                    // failing nodes are dropped so we don't burn queries on them.
                    // Fall back to ready-only (any interval-elapsed, freshest
                    // available) for cold-start coverage so a fresh table still
                    // gets sampled before first responses arrive.
                    let pool: Vec<(Id20, SocketAddr)> = {
                        let ns = node_stats.lock().unwrap();
                        let fresh: Vec<(Id20, SocketAddr)> = ready
                            .iter()
                            .copied()
                            .filter(|(_, a)| ns.is_sampling_fresh(a, now))
                            .filter(|(_, a)| !ns.is_recently_failing(a, now))
                            .collect();
                        let proven: Vec<(Id20, SocketAddr)> = fresh
                            .iter()
                            .copied()
                            .filter(|(_, a)| ns.is_bep51_capable(a))
                            .collect();
                        if !proven.is_empty() {
                            proven
                        } else if !fresh.is_empty() {
                            fresh
                        } else {
                            // Cold start / all nodes unproven-fresh: fall back to
                            // ready nodes, excluding recently-failing ones.
                            ready
                                .iter()
                                .copied()
                                .filter(|(_, a)| !ns.is_recently_failing(a, now))
                                .collect()
                        }
                    };
                    {
                        let mut q = candidates.lock().unwrap();
                        if q.len() < CANDIDATE_QUEUE_CAP {
                            for c in pool.iter().take(CANDIDATE_BATCH) {
                                q.push_back(*c);
                            }
                        }
                    }
                }
            });
        }
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
                min_sightings: self.cfg.min_sightings.max(1),
                min_seen_shadow: self.cfg.min_seen_shadow,
                max_interval,
                cursor: 0,
                sought_rx: self.sought_rx.clone(),
                shared: self.shared.clone(),
                seen_bloom: self.seen.clone(),
                liveness: self.liveness.clone(),
                shutdown: self.shutdown.clone(),
                candidates: candidates.clone(),
            };
            tasks.spawn(async move { loop_.run_loop().await });
        }
        while tasks.join_next().await.is_some() {}

        // Persist per-node sampler state to Redis on shutdown so the next start
        // resumes with the same intervals/quality (no file persistence).
        let interval_rows: Vec<(String, i64, u64)> = {
            let iv = intervals.lock().unwrap();
            let now = Instant::now();
            iv.map
                .iter()
                .map(|(addr, (last, interval))| {
                    (
                        addr.to_string(),
                        now.duration_since(*last).as_secs() as i64,
                        interval.as_secs(),
                    )
                })
                .collect()
        };
        let quality_rows: Vec<(String, u64, u64, u32, bool)> = {
            let ns = node_stats.lock().unwrap();
            ns.map
                .iter()
                .map(|(addr, s)| {
                    (
                        addr.to_string(),
                        s.samples,
                        s.failures,
                        s.consecutive_stale,
                        s.bep51_capable,
                    )
                })
                .collect()
        };
        self.shared.sampler_intervals_put(&interval_rows).await;
        self.shared.sampler_quality_put(&quality_rows).await;
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
    min_sightings: u32,
    min_seen_shadow: Option<u32>,
    max_interval: Duration,
    shared: crate::redis::SharedState,
    seen_bloom: crate::bloom::SharedBloom,
    liveness: Arc<crate::discovery::LivenessCounter>,
    /// Rotating cursor over the routing table: each pick advances past the
    /// previous node so loops cycle through the whole table instead of
    /// re-selecting the same high-score nodes (the "no ready node" starvation).
    cursor: usize,
    /// Shared rotating `sample_infohashes` target (bitmagnet's `soughtNodeID`).
    /// All loops read the same value, which a background task rotates every
    /// ~10s. Bitmagnet uses this shared target for every node: it returns a
    /// DIFFERENT random slice of each node's store per rotation, so re-querying
    /// a node as its backoff expires surfaces NEW hashes each time. Own-ID
    /// targeting (tested) returns the same home-cluster hashes repeatedly and
    /// collapses the unique rate.
    sought_rx: tokio::sync::watch::Receiver<Id20>,
    shutdown: CancellationToken,
    /// Shared candidate queue populated by the background feeder (see
    /// `Sampler::run`). Loops pop one candidate per iteration instead of
    /// re-scanning the whole routing table.
    candidates: Arc<std::sync::Mutex<std::collections::VecDeque<(Id20, SocketAddr)>>>,
}

impl SamplerLoop {
    async fn run_loop(&mut self) {
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            // Pop one ready candidate from the shared feeder queue (bitmagnet
            // channel model) — no per-loop routing-table re-scan.
            let node_addr = {
                let next = { self.candidates.lock().unwrap().pop_front() };
                match next {
                    Some((_, addr)) => addr,
                    None => {
                        tokio::time::sleep(MIN_LOOP_DELAY).await;
                        continue;
                    }
                }
            };
            let now = Instant::now();

            self.gate.acquire().await;
            // Bitmagnet's shared `soughtNodeID`: every query uses the same
            // rotating target so each node returns a random slice of its store
            // for that region, and re-queries as backoff expires surface new
            // hashes. Own-ID targeting (tested) hit the same home-cluster hashes
            // repeatedly and collapsed the unique rate.
            let sought = *self.sought_rx.borrow();
            let result = tokio::time::timeout(
                SAMPLE_TIMEOUT,
                self.handle.sample_infohashes_from(node_addr, sought),
            )
            .await;
            let result = match result {
                Ok(r) => r,
                Err(_elapsed) => {
                    debug!(%node_addr, "sample_infohashes hung, timed out");
                    // A non-responsive node gets a longer backoff than a healthy
                    // 0-new node — it may be offline. Reset the stale counter.
                    self.intervals.lock().unwrap().record(node_addr, FAIL_BACKOFF, now);
                    self.node_stats.lock().unwrap().record_hang_locked(node_addr, now);
                    continue;
                }
            };
            match result {
                Ok(res) => {
                    let advertised = Duration::from_secs(res.interval.max(0) as u64);
                    let capped = advertised.min(self.max_interval);
                    let total_samples = res.samples.len();
                    debug!(
                        %node_addr,
                        interval_secs = res.interval,
                        capped = capped.as_secs(),
                        samples = total_samples,
                        closer_nodes = res.nodes.len(),
                        "sample_infohashes ok"
                    );
                    let mut new_count = 0u32;
                    // Batch pre-filter: bloom + a single batched DB check for
                    // terminal (ok/skipped) hashes, so the per-hash hot loop
                    // below only awaits DB/Redis for the survivors. Semantics
                    // are unchanged: terminal verdicts are cached in the bloom
                    // exactly as `emit_sample` would; Failed/backoff rows still
                    // flow through `emit_sample`'s own scan_status per hash.
                    let bloom_miss: Vec<Id20> = res
                        .samples
                        .iter()
                        .filter(|h| !self.seen_bloom.contains(h.as_bytes()))
                        .copied()
                        .collect();
                    let terminal = self
                        .storage
                        .scan_statuses_batch(
                            &bloom_miss.iter().map(|h| *h.as_bytes()).collect::<Vec<_>>(),
                        )
                        .await
                        .unwrap_or_default();
                    for h in &bloom_miss {
                        if terminal.contains_key(h.as_bytes()) {
                            self.seen_bloom.insert(h.as_bytes());
                        }
                    }
                    for sample in res.samples {
                        // The shared rotating `sought` target is the liveness-
                        // counter source (the same for all loops, matching
                        // bitmagnet's shared `soughtNodeID`); `node_addr` is the
                        // reporting node, used as a fetch lookup seed. Terminal
                        // hashes (pre-cached above) short-circuit inside
                        // emit_sample's bloom check without a per-hash DB query.
                        match self.emit_sample(sample, sought, node_addr).await {
                            EmitOutcome::Shutdown => return,
                            EmitOutcome::Repeat => {}
                            EmitOutcome::New => new_count += 1,
                        }
                    }
                    // Bitmagnet's backoff semantics exactly. Any successful response
                    // proves BEP 51 capability; keep the quality ledger updated.
                    self.node_stats
                        .lock()
                        .unwrap()
                        .record_result_locked(node_addr, total_samples, now);
                                        // * node with NEW hashes → re-query soon (capped at 60s)
                    //  * empty node → honor the FULL advertised interval.
                    //    Bitmagnet relies on its unbounded discoveredNodes
                    //    injection to keep the sampler fed while empty nodes
                    //    shelve for 6h. Our bounded table (~200k nodes) would
                    //    run completely dry if every empty node shelved 6h —
                    //    the feeder's ready pool collapses and the sampler
                    //    stalls (measured: unique_per_hr → 0). So for us the
                    //    empty shelf is capped at 60s: the shared sought target
                    //    rotates every 10s, so a node re-queried after 60s faces
                    //    a DIFFERENT region and can yield new hashes. This is
                    //    the config that sustained 90-115 samples/sec.
                    if new_count > 0 {
                        self.intervals
                            .lock()
                            .unwrap()
                            .record(node_addr, advertised.min(self.max_interval).max(RESAMPLE_FLOOR), now);
                    } else {
                        self.intervals
                            .lock()
                            .unwrap()
                            .record(node_addr, advertised.min(self.max_interval).max(RESAMPLE_FLOOR), now);
                    }
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
                    self.node_stats.lock().unwrap().record_failure_locked(node_addr, now);
                    // Bitmagnet's DropNode: a node that fails sample_infohashes
                    // can't feed the pipeline. Remove it so the routing table
                    // keeps room for fresh, reachable discoveries. Random-index
                    // requires &mut; the task owns its borrows, so warn_allow:
                    // the failure path is rare and this avoids holding the
                    // candidates lock across the await.
                    let _ = self.handle.remove_node(node_addr).await;
                }
            }
        }
    }

    /// Route a sampled hash into the fetch pipeline. This is bitmagnet's
    /// `runSampleInfoHashes` + `infoHashTriage` semantics: the ONLY dedup is
    /// an in-memory bloom `testAndAdd` (bitmagnet's `ignoreHashes`) — no Redis
    /// round-trips, no liveness gate. Bitmagnet fetches on FIRST sighting; a
    /// repeated hash is skipped by the bloom. Terminal-verdict hashes are
    /// already cached into the bloom by the batch pre-filter in the response
    /// loop (bitmagnet's batched DB triage), so an already-indexed hash is
    /// skipped here without a per-hash DB query.
    async fn emit_sample(&mut self, hash: Id20, source: Id20, report_addr: SocketAddr) -> EmitOutcome {
        self.stats
            .hashes_sampled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Bitmagnet `ignoreHashes.testAndAdd`: atomic in-memory check+add.
        // True = already seen (repeat or terminal), skip. False = new.
        if self.seen_bloom.test_and_add(hash.as_bytes()) {
            return EmitOutcome::Repeat;
        }

        // Record the report for shadow-mode observation (bitmagnet has no
        // liveness gate; this is purely diagnostic and does not gate).
        self.liveness.record(hash.as_bytes(), source, Instant::now());

        self.stats
            .hashes_unique
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ok = self
            .emit
            .send(FetchRequest {
                hash,
                occurrences: 1,
                peer_hint: None,
                source: crate::discovery::FetchSource::Sampled,
                lookup_seed: Some(report_addr),
                dht_handle: Some(self.handle.clone()),
            })
            .await
            .is_ok();
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
    let iv = intervals.lock().unwrap();
    let ns = node_stats.lock().unwrap();
    let ready: Vec<(Id20, SocketAddr)> = nodes
        .iter()
        .filter(|(_, a)| iv.is_ready(a, now))
        .map(|(id, addr)| (*id, *addr))
        .collect();
    if ready.is_empty() {
        return None;
    }
    // Bitmagnet's `IsSampleInfoHashesCandidate` concentrates the sampling
    // budget on nodes that have proven BEP 51 capable. GAIA's equivalent:
    // when any ready node is BEP51 capable, restrict the candidate pool to
    // those proven producers so the sampler isn't diluted by unknown nodes.
    // Unknown nodes are still probed whenever nothing proven is ready
    // (cold-start coverage). The proven pool can be empty even when
    // `has_any_proven()` is true (the productive nodes are all cooling), so
    // fall back to the full ready list.
    let mut pool: Vec<(Id20, SocketAddr)> = if ns.has_any_proven() {
        let proven: Vec<(Id20, SocketAddr)> = ready
            .iter()
            .copied()
            .filter(|(_, a)| ns.is_bep51_capable(a))
            .collect();
        if proven.is_empty() { ready } else { proven }
    } else {
        ready
    };
    // Rotate: advance the cursor past the previously-picked node so we don't
    // keep landing on the same spot of the ready list.
    *cursor = cursor.checked_add(1).unwrap_or(0) % pool.len().max(1);
    let rot = *cursor % pool.len();
    // Rotate the ready list so the scan window starts at the cursor. We do NOT
    // shuffle: shuffling right after the rotation negates the cursor and
    // re-randomizes every pick. Within the window, pick randomly among the
    // top-N scored nodes so lower-scored but still productive nodes get picked
    // instead of always choosing the single global best.
    pool.rotate_left(rot);
    let mut scored: Vec<(i64, Id20, SocketAddr)> = pool
        .iter()
        .take(PICK_CANDIDATES)
        .map(|(id, addr)| (ns.score_locked(addr), *id, *addr))
        .collect();
    // Sort descending by score, keep the top TOP_N (at least 1), then pick
    // one at random from that shortlist.
    const TOP_N: usize = 8;
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    let shortlist_len = scored.len().clamp(1, TOP_N);
    let pick = thread_rng().gen_range(0..shortlist_len);
    scored
        .get(pick)
        .map(|(_, id, addr)| (*id, *addr))
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn random_id20() -> Id20 {
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    Id20(bytes)
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
        // With more ready nodes than the TOP_N shortlist, a clearly-penalized
        // node (ranked worst) must be excluded from the shortlist and never
        // picked, while the best-scoring node is always in the shortlist.
        let mut node_stats = NodeStats::new(64);
        let mut nodes: Vec<(Id20, SocketAddr)> = Vec::new();
        for i in 0..12u8 {
            let addr: SocketAddr = format!("127.0.0.1:{i}").parse().unwrap();
            node_stats.ensure_locked(addr);
            nodes.push((id(i), addr));
        }
        let best_addr: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let worst_addr: SocketAddr = "127.0.0.1:3".parse().unwrap();
        node_stats.map.get_mut(&best_addr).unwrap().samples = 50;
        node_stats.map.get_mut(&worst_addr).unwrap().failures = 100;

        let intervals = Arc::new(std::sync::Mutex::new(IntervalMap::new(64)));
        let node_stats = Arc::new(std::sync::Mutex::new(node_stats));
        let now = Instant::now();
        let mut cursor = 0usize;
        for _ in 0..50 {
            let (_, addr) = pick_target(&mut cursor, &intervals, &node_stats, &nodes, now)
                .unwrap();
            assert_ne!(addr, worst_addr, "worst-scored node must never be picked");
        }
    }

    #[test]
    fn pick_target_falls_back_when_proven_pool_empty() {
        // Regression: when `has_any_proven()` is true but every proven node is
        // cooling (none ready), the filtered pool must not be empty — a
        // modulo-by-zero in the rotation previously panicked all sampler loops.
        let mut intervals = IntervalMap::new(16);
        let mut node_stats = NodeStats::new(16);
        let now = Instant::now();
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        // `a` is proven but cooling; `b` is unproven and ready.
        node_stats.ensure_locked(a);
        node_stats.map.get_mut(&a).unwrap().samples = 5;
        intervals.record(a, Duration::from_secs(60), now);
        let nodes = vec![(id(1), a), (id(2), b)];

        let iv = Arc::new(std::sync::Mutex::new(intervals));
        let ns = Arc::new(std::sync::Mutex::new(node_stats));
        let mut cursor = 0usize;
        let picked = pick_target(&mut cursor, &iv, &ns, &nodes, now)
            .expect("ready unproven node must still be pickable");
        assert_eq!(picked.1, b);
    }
}
