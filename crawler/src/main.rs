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
///
/// Notes on correctness vs the live crawler:
/// - `VACUUM INTO` opens a read transaction; with an active WAL it yields the
///   database as of the transaction start, checkpointing the WAL into the
///   output. To avoid `SQLITE_BUSY` when the crawler holds a write lock, set a
///   generous `busy_timeout` — otherwise a 900 MB live DB fails or produces a
///   partial file (this is what made earlier snapshots "malformed").
/// - The output file is standalone (no WAL); `docker cp` of it is race-free
///   only if the copy happens after this returns (it does).
fn snapshot(args: &cli::SnapshotArgs) -> Result<()> {
    if std::path::Path::new(&args.out).exists() {
        std::fs::remove_file(&args.out)
            .with_context(|| format!("remove existing snapshot {}", args.out))?;
    }
    let conn = rusqlite::Connection::open(&args.db)
        .with_context(|| format!("open db {}", args.db))?;
    // The live crawler writes in WAL mode; this reader must tolerate brief
    // write-lock holds. 30s is well beyond any single write transaction.
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .with_context(|| "set busy timeout")?;
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
