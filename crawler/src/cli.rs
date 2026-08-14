use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "crawler", about = "DHT torrent crawler indexing movie/TV torrents into SQLite")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the crawl daemon (DHT sampler + metadata fetcher + storage).
    Run(Box<RunArgs>),
    /// Search the local database for torrent names.
    Query(QueryArgs),
    /// Delete the database and routing state so the next run starts fresh.
    Purge(PurgeArgs),
    /// Write a consistent snapshot of the database via `VACUUM INTO` (online
    /// backup; safe while the crawler is running). Output is a standalone DB
    /// with no WAL, ideal for offline benchmark/analysis replays.
    Snapshot(SnapshotArgs),
    /// Replay a fetch/peer-resolution strategy against a DB snapshot: sample
    /// hashes by their recorded outcome and measure peers-found / verified
    /// without a full deploy cycle.
    BenchFetch(BenchFetchArgs),
}

#[derive(Debug, Args)]
pub struct SnapshotArgs {
    /// PostgreSQL connection URL.
    #[arg(long)]
    pub pg: String,

    /// Destination path for the snapshot (pg_dump custom format).
    #[arg(long)]
    pub out: String,
}

#[derive(Debug, Args)]
pub struct BenchFetchArgs {
    /// PostgreSQL connection URL to sample from.
    #[arg(long)]
    pub pg: String,

    /// Number of hashes to sample per outcome class.
    #[arg(long, default_value_t = 50)]
    pub sample: usize,

    /// Which outcome class to sample. One of: empty_peers, timeout, other,
    /// deadline, ok, all. "ok" samples previously-verified hashes (control).
    #[arg(long, default_value = "empty_peers")]
    pub class: String,

    /// If set, dial tracker-resolved peers and attempt metadata verification
    /// (network-bound). If unset, only measures tracker peer resolution.
    #[arg(long)]
    pub verify: bool,

    /// Concurrency for the benchmark probes.
    #[arg(long, default_value_t = 16)]
    pub concurrency: usize,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// UDP port to bind the DHT node to.
    #[arg(long, default_value_t = 6881)]
    pub port: u16,

    /// PostgreSQL connection URL (the single store).
    #[arg(long, default_value = "postgres://crawler:crawler@localhost:5432/crawler")]
    pub pg: String,

    /// Maximum concurrent in-flight metadata fetches.
    #[arg(long, default_value_t = 512)]
    pub concurrency: usize,

    /// Enable IPv6 DHT support.
    #[arg(long)]
    pub ipv6: bool,

    /// Directory for persisting the DHT routing table.
    #[arg(long, default_value = "state")]
    pub state_dir: PathBuf,

    /// Number of independent DHT nodes/samplers to run, sharing one database.
    /// Instance i binds UDP port `port+i` and uses `state-dir/instance-i/`.
    #[arg(long, default_value_t = 1)]
    pub instances: usize,

    /// Comma-separated bootstrap nodes (host:port).
    #[arg(long, value_delimiter = ',', default_value = "router.bittorrent.com:6881,dht.transmissionbt.com:6881,router.utorrent.com:6881,dht.libtorrent.org:25401,dht.aelitis.com:6881,router.bitcomet.com:6881,bt.offer.bitcomet.com:6881,router.bittorrent.com:6882")]
    pub bootstrap: Vec<String>,

    /// Aggregate DHT query budget (per second) shared by sampling and peer
    /// lookups. Must comfortably exceed the sampler rate.
    #[arg(long, default_value_t = 2000)]
    pub qps: usize,

    /// Sampler aggregate query budget (per second) across all sampling loops.
    #[arg(long, default_value_t = 400)]
    pub sampler_qps: usize,

    /// Number of concurrent sampling loops sharing the sampler budget.
    #[arg(long, default_value_t = 32)]
    pub sampler_loops: usize,

    /// Concurrency scale factor (bitmagnet's `scaling_factor`). Multiplies the
    /// effective sampler QPS, sampler loops, fetch concurrency, lookup
    /// concurrency, and pipeline buffer sizes. Default 10 matches bitmagnet's
    /// proven baseline; raise to 50 for aggressive day-one aggregation.
    #[arg(long, default_value_t = 10)]
    pub scale: usize,

    /// Emit an infohash to the fetcher after this many distinct sampling
    /// responses reported it (1 = fetch on first sighting; the in-memory bloom
    /// filter already prevents re-fetching hashes confirmed Ok/Skipped).
    #[arg(long, default_value_t = 1)]
    pub min_seen: u32,

    /// Sparse/stalled discriminator: when ≥ 2, a single-source hash is emitted
    /// only after the SAME source re-reports it this many times (its node kept
    /// refreshing = live), instead of on a first sighting from a backoff-stalled
    /// node. Corroboration (≥ 2 distinct sources) still emits immediately.
    #[arg(long, default_value_t = 1)]
    pub min_sightings: u32,

    /// Optional liveness-gate shadow threshold: observe what `--min-seen` would
    /// filter (log `shadow_filtered`/`shadow_emitted`/near-miss counters) while
    /// the live path keeps emitting at `--min-seen`. 0 = disabled.
    #[arg(long, default_value_t = 0)]
    pub min_seen_shadow: u32,

    /// Rolling window (seconds) for the liveness gate: a distinct-source report
    /// older than this expires.
    #[arg(long, default_value_t = 120)]
    pub liveness_window_secs: u64,

    /// Max distinct sources tracked per hash before the oldest is evicted.
    #[arg(long, default_value_t = 8)]
    pub liveness_cap: usize,

    /// Global liveness entry backstop (oldest evicted past this).
    #[arg(long, default_value_t = 100_000)]
    pub liveness_max_entries: usize,

    /// Upper bound (seconds) on the per-node re-query interval advertised by
    /// BEP 51 nodes. Nodes reporting longer intervals are re-queried after
    /// this period so the routing table keeps growing.
    #[arg(long, default_value_t = 60)]
    pub sampler_max_interval: u64,

    /// A failed fetch is retried until this many attempts, then treated as a
    /// terminal dead hash (cached in the in-process bloom, never re-emitted).
    /// Dead hashes never recover (F-series), so retries are wasted fetch work.
    #[arg(long, default_value_t = 2)]
    pub max_attempts: u32,

    /// Maximum concurrent DHT `get_peers` lookups (bounds query-budget use).
    #[arg(long, default_value_t = 256)]
    pub lookup_concurrency: usize,

    /// Maximum number of nodes in the DHT routing table.
    #[arg(long, default_value_t = 4096)]
    pub max_nodes: usize,

    /// Disable irontide's one-node-per-IP routing restriction. Off by default;
    /// enable on NAT hosts where many peers share egress IPs and routing
    /// diversity is suppressed.
    #[arg(long)]
    pub no_restrict_ips: bool,

    /// Timeout in seconds for individual DHT queries. 3s drains slow/dead
    /// queries faster, reducing the in-flight KRPC backlog (`pending_queries`)
    /// and the memory held by long-lived lookups.
    #[arg(long, default_value_t = 3)]
    pub query_timeout: u64,

    /// Aggressive preset: crank up sampling and fetch rates for VPS deployments.
    /// Overrides --sampler-qps, --sampler-loops, --concurrency,
    /// --lookup-concurrency, --dht-qps, --max-nodes, --query-timeout.
    #[arg(long)]
    pub aggressive: bool,

    /// Optional blocklist file: one IP or CIDR per line, '#' comments allowed.
    #[arg(long)]
    pub blocklist: Option<PathBuf>,

    /// Optional Redis URL (e.g. redis://redis:6379) for a shared seen-set and
    /// dead-peer cache across instances. If absent or unreachable, the crawler
    /// falls back to per-instance in-memory dedup/cache.
    #[arg(long)]
    pub redis_url: Option<String>,

    /// Override the RUST_LOG tracing filter (e.g. "crawler=debug").
    #[arg(long)]
    pub log: Option<String>,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    /// Substring to search for in torrent names.
    pub name: String,

    /// PostgreSQL connection URL.
    #[arg(long, default_value = "postgres://crawler:crawler@localhost:5432/crawler")]
    pub pg: String,

    /// Instead of searching names, print a breakdown of metadata fetch
    /// failures by `failure_reason` from the `scanned` table.
    #[arg(long)]
    pub failures: bool,
}

#[derive(Debug, Args)]
pub struct PurgeArgs {
    /// PostgreSQL connection URL.
    #[arg(long, default_value = "postgres://crawler:crawler@localhost:5432/crawler")]
    pub pg: String,

    /// Directory holding the persisted DHT routing table.
    #[arg(long, default_value = "state")]
    pub state_dir: PathBuf,

    /// Skip asking for confirmation.
    #[arg(long)]
    pub yes: bool,
}

impl RunArgs {
    /// The concurrency scale factor, clamped to a sane range.
    fn scale(&self) -> usize {
        self.scale.max(1)
    }

    /// Effective sampler QPS after applying the aggressive preset and `--scale`.
    /// Capped at 800: the sampler can only usefully query distinct nodes
    /// (bounded by table size / re-query interval), but a ceiling below the
    /// real query demand starved discovery after the leak fixes. 800 lets the
    /// backoff-inversion + rotating-cursor spread reach more of the table.
    pub fn effective_sampler_qps(&self) -> usize {
        let base = if self.aggressive { 1000 } else { self.sampler_qps };
        base.saturating_mul(self.scale()).min(800)
    }

    /// Effective sampler loops after applying the aggressive preset and `--scale`.
    /// Capped at 64: enough loops to spread sampling across the routing table
    /// (with the rotating cursor) without per-loop overhead dominating.
    pub fn effective_sampler_loops(&self) -> usize {
        let base = if self.aggressive { 64 } else { self.sampler_loops };
        base.saturating_mul(self.scale()).min(64)
    }

    /// Effective concurrency after applying the aggressive preset and `--scale`.
    pub fn effective_concurrency(&self) -> usize {
        let base = if self.aggressive { 512 } else { self.concurrency };
        base.saturating_mul(self.scale())
    }

    /// Effective lookup concurrency after applying the aggressive preset and `--scale`.
    /// Capped at 384 (was 96): the 96 ceiling starved concurrent get_peers
    /// lookups, capping discovery. 384 is ample for the fetch pool while the
    /// leak fixes (64-node lookups, fast-exit, bounded announce_tokens) keep
    /// active_lookups memory controlled.
    pub fn effective_lookup_concurrency(&self) -> usize {
        let base = if self.aggressive { 256 } else { self.lookup_concurrency };
        base.saturating_mul(self.scale()).min(384)
    }

    /// Effective DHT QPS after applying the aggressive preset.
    pub fn effective_qps(&self) -> usize {
        if self.aggressive { 4000 } else { self.qps }
    }

    /// Effective max routing nodes after applying the aggressive preset.
    pub fn effective_max_nodes(&self) -> usize {
        if self.aggressive { 8192 } else { self.max_nodes }
    }

    /// Effective query timeout after applying the aggressive preset.
    pub fn effective_query_timeout(&self) -> u64 {
        if self.aggressive { 3 } else { self.query_timeout }
    }

    /// Effective min_seen after applying the aggressive preset.
    pub fn effective_min_seen(&self) -> u32 {
        if self.aggressive { self.min_seen.max(2) } else { self.min_seen }
    }
}
