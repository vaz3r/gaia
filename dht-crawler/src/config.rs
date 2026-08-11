use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "dht-crawler", about = "DHT torrent crawler indexing movie/TV torrents into SQLite")]
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

    /// Comma-separated bootstrap nodes (host:port).
    #[arg(long, value_delimiter = ',', default_value = "router.bittorrent.com:6881,dht.transmissionbt.com:6881,router.utorrent.com:6881,dht.libtorrent.org:25401")]
    pub bootstrap: Vec<String>,

    /// Aggregate DHT query budget (per second) shared by sampling and peer
    /// lookups. Must comfortably exceed the sampler rate.
    #[arg(long, default_value_t = 2000)]
    pub qps: usize,

    /// Sampler aggregate query budget (per second) across all sampling loops.
    #[arg(long, default_value_t = 240)]
    pub sampler_qps: usize,

    /// Number of concurrent sampling loops sharing the sampler budget.
    #[arg(long, default_value_t = 4)]
    pub sampler_loops: usize,

    /// Emit an infohash to the fetcher only after this many distinct sampling
    /// responses reported it (1 = no culling).
    #[arg(long, default_value_t = 1)]
    pub min_seen: u32,

    /// Maximum concurrent DHT `get_peers` lookups (bounds query-budget use).
    #[arg(long, default_value_t = 32)]
    pub lookup_concurrency: usize,

    /// Optional blocklist file: one IP or CIDR per line, '#' comments allowed.
    #[arg(long)]
    pub blocklist: Option<PathBuf>,

    /// Override the RUST_LOG tracing filter (e.g. "dht_crawler=debug").
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
}

impl RunArgs {
    /// The UDP bind address for this runtime, honoring `--ipv6`.
    pub fn bind_addr(&self) -> SocketAddr {
        if self.ipv6 {
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, self.port))
        } else {
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, self.port))
        }
    }
}
