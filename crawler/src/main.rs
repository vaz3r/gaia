mod bench;
mod cli;
mod bloom;
mod crawler;
mod db;
mod discovery;
mod fetch;
mod health;
mod net;
mod purge;
mod query;
mod redis;
mod retry;
mod stats;
mod storage;
mod sysmetrics;

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
        Command::Query(args) => query::query(args).await,
        Command::Purge(args) => purge::purge(&args).await,
        Command::Snapshot(args) => snapshot(&args).await,
        Command::BenchFetch(args) => bench::run(&args).await,
    }
}

/// Consistent Postgres snapshot via `pg_dump` (custom format), safe to run
/// while the crawler writes. Produces a standalone dump for offline
/// benchmark/analysis replays.
async fn snapshot(args: &cli::SnapshotArgs) -> Result<()> {
    if std::path::Path::new(&args.out).exists() {
        std::fs::remove_file(&args.out)
            .with_context(|| format!("remove existing snapshot {}", args.out))?;
    }
    let status = tokio::process::Command::new("pg_dump")
        .arg(args.pg.clone())
        .arg("--format=custom")
        .arg("--no-owner")
        .arg("--file")
        .arg(&args.out)
        .status()
        .await
        .with_context(|| "run pg_dump (is postgresql-client installed?)")?;
    if !status.success() {
        anyhow::bail!("pg_dump failed with status {status}");
    }
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
