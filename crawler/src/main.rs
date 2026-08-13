mod bench;
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

use anyhow::{Context, Result};
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
        Command::Snapshot(args) => snapshot(&args),
        Command::BenchFetch(args) => bench::run(&args).await,
    }
}

/// `VACUUM INTO` online backup: a consistent, WAL-free snapshot of the live
/// database, safe to run while the crawler writes. Used to produce a clean DB
/// copy for offline benchmark replays (the direct tar/cp snapshot of a live
/// WAL-mode DB is not consistent).
fn snapshot(args: &cli::SnapshotArgs) -> Result<()> {
    if std::path::Path::new(&args.out).exists() {
        std::fs::remove_file(&args.out)
            .with_context(|| format!("remove existing snapshot {}", args.out))?;
    }
    let conn = rusqlite::Connection::open(&args.db)
        .with_context(|| format!("open db {}", args.db))?;
    conn.execute_batch(&format!(
        "VACUUM INTO '{}';",
        args.out.replace('\'', "''")
    ))
    .with_context(|| format!("vacuum into {}", args.out))?;
    tracing::info!(out = %args.out, "snapshot written");
    Ok(())
}

fn init_tracing(log: &Option<String>) {
    let filter = match log {
        Some(filter) => EnvFilter::new(filter),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
