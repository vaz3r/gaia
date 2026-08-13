mod cli;
mod bloom;
mod crawler;
mod discovery;
mod fetch;
mod net;
mod purge;
mod query;
mod redis;
mod stats;
mod storage;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};

// Global allocator: jemalloc. Diagnostics via MALLOC_CONF=stats_print:true
// (dumps allocator stats on exit) — used to attribute RSS growth. The leaked
// allocation is bounded by this allocator; its stats name the live regions.
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => {
            init_tracing(&args.log);
            crawler::run(*args).await
        }
        Command::Query(args) => query::query(args),
        Command::Purge(args) => purge::purge(&args),
    }
}

fn init_tracing(log: &Option<String>) {
    let filter = match log {
        Some(filter) => EnvFilter::new(filter),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
