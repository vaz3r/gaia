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
    Run(RunArgs),
    /// Search the local database for torrent names.
    Query(QueryArgs),
    /// Delete the database and routing state so the next run starts fresh.
    Purge(PurgeArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// UDP port to bind the DHT node to.
    #[arg(long, default_value_t = 6881)]
    pub port: u16,

    /// SQLite database file path.
    #[arg(long, default_value = "crawler.sqlite")]
    pub db: String,

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

    /// Upper bound (seconds) on the per-node re-query interval advertised by
    /// BEP 51 nodes. Nodes reporting longer intervals are re-queried after
    /// this period so the routing table keeps growing.
    #[arg(long, default_value_t = 60)]
    pub sampler_max_interval: u64,

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

    /// Timeout in seconds for individual DHT queries.
    #[arg(long, default_value_t = 5)]
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

    /// SQLite database file path.
    #[arg(long, default_value = "crawler.sqlite")]
    pub db: String,

    /// Instead of searching names, print a breakdown of metadata fetch
    /// failures by `failure_reason` from the `scanned` table.
    #[arg(long)]
    pub failures: bool,
}

#[derive(Debug, Args)]
pub struct PurgeArgs {
    /// SQLite database file path.
    #[arg(long, default_value = "crawler.sqlite")]
    pub db: String,

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

    /// Effective sampler QPS after applying the aggressive preset and `--scale`
    /// (bitmagnet `scaling_factor`).
    pub fn effective_sampler_qps(&self) -> usize {
        let base = if self.aggressive { 1000 } else { self.sampler_qps };
        base.saturating_mul(self.scale())
    }

    /// Effective sampler loops after applying the aggressive preset and `--scale`.
    pub fn effective_sampler_loops(&self) -> usize {
        let base = if self.aggressive { 64 } else { self.sampler_loops };
        base.saturating_mul(self.scale())
    }

    /// Effective concurrency after applying the aggressive preset and `--scale`.
    pub fn effective_concurrency(&self) -> usize {
        let base = if self.aggressive { 512 } else { self.concurrency };
        base.saturating_mul(self.scale())
    }

    /// Effective lookup concurrency after applying the aggressive preset and `--scale`.
    pub fn effective_lookup_concurrency(&self) -> usize {
        let base = if self.aggressive { 256 } else { self.lookup_concurrency };
        base.saturating_mul(self.scale())
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
