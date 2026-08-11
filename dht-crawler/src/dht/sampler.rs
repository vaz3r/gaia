use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irontide_core::Id20;
use irontide_dht::DhtHandle;
use rand::seq::SliceRandom;
use rand::thread_rng;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::stats::CrawlStats;
use crate::storage::Storage;

/// How often a node that failed to answer is retried.
const FAIL_BACKOFF: Duration = Duration::from_secs(10);
/// Cap on the per-node interval map (LRU-evicted).
const INTERVAL_MAP_CAP: usize = 8192;
/// Cap on the in-memory occurrence map (FIFO-evicted).
const SEEN_CAP: usize = 1_000_000;
/// Cap on the per-node quality map (LRU-evicted).
const NODE_STATS_CAP: usize = 32_768;
/// Minimum time to wait when no node is re-queryable.
const MIN_LOOP_DELAY: Duration = Duration::from_millis(10);
/// Time to wait for the routing table to populate before warning.
const BOOTSTRAP_WAIT: Duration = Duration::from_secs(15);
/// Number of random ready nodes to sample when picking a target; the highest
/// quality among them wins, spreading queries across the routing table.
const PICK_CANDIDATES: usize = 32;
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
    /// Upper bound (seconds) on the per-node re-query interval advertised by
    /// BEP 51 nodes. Nodes that report longer intervals are still re-queried
    /// after this period so the routing table keeps growing.
    pub max_interval_secs: u64,
}

/// A distinct infohash emitted into the fetch pipeline together with the
/// number of sampling responses that reported it (its popularity signal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampledHash {
    pub hash: Id20,
    pub occurrences: u32,
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

/// Bounded, FIFO-evicted hash → distinct-reporting-nodes counter.
struct SeenCounts {
    map: HashMap<Id20, u32>,
    order: VecDeque<Id20>,
    cap: usize,
}

impl SeenCounts {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    /// Bump the count for `h` and return it. Evicts the oldest entry past cap.
    fn record(&mut self, h: Id20) -> u32 {
        let new = !self.map.contains_key(&h);
        let count = {
            let entry = self.map.entry(h).or_insert(0);
            *entry += 1;
            *entry
        };
        if new {
            self.order.push_back(h);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
        count
    }
}

/// Per-node sampling quality for biasing target selection toward productive
/// (BEP 51 capable) nodes.
struct NodeStat {
    samples: u64,
    failures: u64,
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

    fn get_mut(&mut self, addr: SocketAddr) -> &mut NodeStat {
        if !self.map.contains_key(&addr) {
            self.order.push_back(addr);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
        self.map
            .entry(addr)
            .or_insert(NodeStat { samples: 0, failures: 0 })
    }

    fn score(&self, addr: &SocketAddr) -> i64 {
        self.map.get(addr).map_or(0, |s| s.score())
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
    emit: mpsc::Sender<SampledHash>,
    storage: Storage,
    stats: Arc<CrawlStats>,
    cfg: SamplerConfig,
    shutdown: CancellationToken,
}

impl Sampler {
    pub fn new(
        handle: DhtHandle,
        emit: mpsc::Sender<SampledHash>,
        storage: Storage,
        cfg: &SamplerConfig,
        stats: Arc<CrawlStats>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            handle,
            emit,
            storage,
            stats,
            cfg: cfg.clone(),
            shutdown,
        }
    }

    /// Run all sampling loops until the emit channel closes (shutdown).
    pub async fn run(&self) {
        self.wait_for_bootstrap().await;

        let max_interval = Duration::from_secs(self.cfg.max_interval_secs.max(1));
        let gate = Arc::new(QpsGate::new(self.cfg.queries_per_second));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..self.cfg.concurrency.max(1) {
            let mut loop_ = SamplerLoop {
                handle: self.handle.clone(),
                emit: self.emit.clone(),
                storage: self.storage.clone(),
                stats: self.stats.clone(),
                intervals: IntervalMap::new(INTERVAL_MAP_CAP),
                seen: SeenCounts::new(SEEN_CAP),
                gate: gate.clone(),
                node_stats: NodeStats::new(NODE_STATS_CAP),
                min_seen: self.cfg.min_seen.max(1),
                max_interval,
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

/// One independent sampling loop with its own interval/cooldown state.
struct SamplerLoop {
    handle: DhtHandle,
    emit: mpsc::Sender<SampledHash>,
    storage: Storage,
    stats: Arc<CrawlStats>,
    intervals: IntervalMap,
    seen: SeenCounts,
    gate: Arc<QpsGate>,
    node_stats: NodeStats,
    min_seen: u32,
    max_interval: Duration,
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
                pick_target(&self.intervals, &self.node_stats, &nodes, now)
            else {
                debug!(
                    ready = nodes.iter().filter(|(_, a)| self.intervals.is_ready(a, now)).count(),
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
                    self.intervals.record(node_addr, FAIL_BACKOFF, now);
                    self.node_stats.get_mut(node_addr).failures += 1;
                    continue;
                }
            };
            match result {
                Ok(res) => {
                    let advertised = Duration::from_secs(res.interval.max(0) as u64);
                    let interval = advertised.min(self.max_interval);
                    self.intervals.record(node_addr, interval, now);
                    self.node_stats
                        .get_mut(node_addr)
                        .samples += res.samples.len() as u64;
                    debug!(
                        %node_addr,
                        interval_secs = res.interval,
                        capped = interval.as_secs(),
                        samples = res.samples.len(),
                        closer_nodes = res.nodes.len(),
                        "sample_infohashes ok"
                    );
                    for sample in res.samples {
                        if !self.emit_sample(sample).await {
                            return; // channel closed → shutdown
                        }
                    }
                    // Response nodes were already fed back into the routing table
                    // by the DHT actor.
                }
                Err(e) => {
                    debug!(error = %e, %node_addr, "sample_infohashes failed");
                    self.intervals
                        .record(node_addr, FAIL_BACKOFF, now);
                    self.node_stats.get_mut(node_addr).failures += 1;
                }
            }
        }
    }

    /// Emit a hash exactly once when its occurrence count reaches `min_seen`.
    /// Returns `false` if the pipeline is shut down.
    async fn emit_sample(&mut self, hash: Id20) -> bool {
        self.stats
            .hashes_sampled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Never queue hashes already accepted/filtered or still in backoff.
        if self.storage.scan_blocked(hash.as_bytes(), unix_secs()).unwrap_or(false) {
            return true;
        }

        let count = self.seen.record(hash);
        if count != self.min_seen {
            return true;
        }

        self.stats
            .hashes_unique
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.emit
            .send(SampledHash { hash, occurrences: count })
            .await
            .is_ok()
    }
}

/// Pick a ready node and query it with a target equal to its own node ID. The
/// DHT actor resolves `sample_infohashes` to `closest(target, 1)`, so a target
/// equal to the node's own ID makes the actor query exactly this node. Ready
/// nodes are shuffled then sampled so each loop spreads across the table (with
/// a small routing table, `choose_multiple` returns items in original order —
/// shuffling first prevents all loops from converging on the same node). The
/// highest-quality candidate among the sampled subset wins. A few cooling nodes
/// cannot starve the sampler because every ready node is a candidate.
fn pick_target(
    intervals: &IntervalMap,
    node_stats: &NodeStats,
    nodes: &[(Id20, SocketAddr)],
    now: Instant,
) -> Option<(Id20, SocketAddr)> {
    let mut rng = thread_rng();
    let mut ready: Vec<(Id20, SocketAddr)> = nodes
        .iter()
        .filter(|(_, a)| intervals.is_ready(a, now))
        .map(|(id, addr)| (*id, *addr))
        .collect();
    ready.shuffle(&mut rng);

    let sample = ready.iter().take(PICK_CANDIDATES);
    let mut best: Option<(i64, Id20, SocketAddr)> = None;
    for (id, addr) in sample {
        let score = node_stats.score(addr);
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
    fn seen_counts_bump_and_evict() {
        let mut s = SeenCounts::new(2);
        assert_eq!(s.record(id(1)), 1);
        assert_eq!(s.record(id(1)), 2, "counts accumulate per hash");
        assert_eq!(s.record(id(2)), 1);
        assert_eq!(s.record(id(3)), 1);
        assert!(!s.map.contains_key(&id(1)), "oldest evicted");
        assert!(s.map.contains_key(&id(2)));
        assert!(s.map.contains_key(&id(3)));
        assert_eq!(s.record(id(2)), 2, "surviving hash keeps counting");
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
        ns.get_mut(a).samples = 10;
        ns.get_mut(b).failures = 3;
        assert!(ns.score(&a) > ns.score(&b));
        assert_eq!(ns.score(&a), 10);
        assert_eq!(ns.score(&b), -6);
    }

    #[test]
    fn node_stats_evicts_oldest() {
        let mut ns = NodeStats::new(2);
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let c: SocketAddr = "127.0.0.1:3".parse().unwrap();
        ns.get_mut(a);
        ns.get_mut(b);
        ns.get_mut(c);
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
        let (target, addr) = pick_target(&intervals, &NodeStats::new(16), &nodes, now).unwrap();
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
        assert!(pick_target(&intervals, &NodeStats::new(16), &nodes, now).is_none());

        // After the interval elapses, a node becomes selectable again.
        let later = now + Duration::from_secs(61);
        assert!(pick_target(&intervals, &NodeStats::new(16), &nodes, later).is_some());
    }

    #[test]
    fn pick_target_prefers_productive_node() {
        let mut node_stats = NodeStats::new(16);
        let a: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:2".parse().unwrap();
        node_stats.get_mut(b).samples = 50;
        node_stats.get_mut(a).failures = 5;

        let intervals = IntervalMap::new(16);
        let nodes = vec![(id(1), a), (id(2), b)];
        let now = Instant::now();
        let (target, addr) = pick_target(&intervals, &node_stats, &nodes, now).unwrap();
        assert_eq!(addr, b, "productive node must be picked over penalized one");
        assert_eq!(target, id(2));
    }
}
