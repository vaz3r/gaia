mod config;
mod dht;
mod filter;
mod metadata;
mod net;
mod stats;
mod storage;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use config::{Cli, Command};
use dht::Sampler;
use irontide_dht::DhtHandle;
use metadata::{run_fetcher, FetcherConfig};
use net::Blocklist;
use stats::CrawlStats;
use storage::{Storage, TorrentRecord};

const SAMPLER_CHANNEL: usize = 8192;
const RECORD_CHANNEL: usize = 4096;
const STATS_INTERVAL: Duration = Duration::from_secs(30);
/// Per-hash in-flight-fetch budget permitted before aborting at shutdown.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run(args).await,
        Command::Query(args) => query(args),
        Command::Purge(args) => purge(&args),
    }
}

fn init_tracing(log: &Option<String>) {
    let filter = match log {
        Some(filter) => EnvFilter::new(filter),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn run(args: config::RunArgs) -> Result<()> {
    init_tracing(&args.log);

    let state_dir = args.state_dir.clone();
    if !state_dir.exists() {
        std::fs::create_dir_all(&state_dir)?;
    }

    let storage = Storage::open(&args.db)?;
    let stats = Arc::new(CrawlStats::default());
    let blocklist = Arc::new(Blocklist::load(args.blocklist.as_deref())?);

    let handle = dht::start_dht(&args, args.state_dir.clone()).await?;
    info!(
        port = args.port,
        ipv6 = args.ipv6,
        db = %args.db,
        state_dir = %state_dir.display(),
        concurrency = args.effective_concurrency(),
        aggressive = args.aggressive,
        "dht crawler starting"
    );

    let (hash_tx, hash_rx) = tokio::sync::mpsc::channel(SAMPLER_CHANNEL);
    let (record_tx, record_rx) = tokio::sync::mpsc::channel(RECORD_CHANNEL);

    let sampler_cfg = dht::SamplerConfig {
        queries_per_second: args.effective_sampler_qps(),
        concurrency: args.effective_sampler_loops(),
        min_seen: args.effective_min_seen(),
        max_interval_secs: args.sampler_max_interval,
    };
    let shutdown = tokio_util::sync::CancellationToken::new();
    let sampler = Sampler::new(
        handle.clone(),
        hash_tx,
        storage.clone(),
        &sampler_cfg,
        stats.clone(),
        shutdown.clone(),
    );
    let sampler_task = tokio::spawn(async move { sampler.run().await });

    let mut fetcher = tokio::spawn(run_fetcher(
        hash_rx,
        record_tx,
        handle.clone(),
        storage.clone(),
        FetcherConfig {
            concurrency: args.effective_concurrency(),
            lookup_concurrency: args.effective_lookup_concurrency(),
            blocklist,
        },
        stats.clone(),
    ));

    let writer = tokio::spawn(write_loop(record_rx, storage.clone(), stats.clone()));

    let stats_task = tokio::spawn(stats_loop(handle.clone(), stats.clone()));

    wait_for_shutdown().await;

    info!("shutdown signal received, draining pipeline");

    // Cancel the sampler loops so they drop their `emit` clones and close the
    // fetch channel; the fetcher then drains its in-flight work internally.
    shutdown.cancel();
    sampler_task.abort();
    let _ = tokio::time::timeout(SHUTDOWN_DRAIN + Duration::from_secs(5), &mut fetcher).await;
    fetcher.abort();
    // Dropping the fetcher aborts its JoinSet; the fetch tasks' `record_tx`
    // clones then drop, closing the write channel so the writer drains.
    let _ = writer.await;
    stats_task.abort();

    // Persist the routing table before exit.
    if let Err(e) = handle.shutdown_and_wait().await {
        error!(error = %e, "failed to persist routing table");
    }
    info!("shutdown complete");
    Ok(())
}

/// Single-threaded storage writer: batches records into transactions. Flushes
/// when the batch fills OR after a short interval, so a slow trickle of found
/// torrents is persisted promptly instead of sitting in memory until shutdown.
async fn write_loop(
    mut rx: tokio::sync::mpsc::Receiver<TorrentRecord>,
    storage: Storage,
    stats: Arc<CrawlStats>,
) {
    const BATCH: usize = 256;
    const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
    let mut batch: Vec<TorrentRecord> = Vec::with_capacity(BATCH);
    loop {
        tokio::select! {
            record = rx.recv() => {
                match record {
                    Some(record) => {
                        batch.push(record);
                        if batch.len() >= BATCH {
                            if let Err(e) = storage.insert_batch(&batch) {
                                error!(error = %e, "storage batch failed");
                            }
                            batch.clear();
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(FLUSH_INTERVAL), if !batch.is_empty() => {
                if let Err(e) = storage.insert_batch(&batch) {
                    error!(error = %e, "storage batch failed");
                }
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        if let Err(e) = storage.insert_batch(&batch) {
            error!(error = %e, "storage final batch failed");
        }
    }
    let _ = stats;
}

async fn stats_loop(handle: DhtHandle, stats: Arc<CrawlStats>) {
    let mut tick = tokio::time::interval(STATS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let routing = handle.node_count().await.unwrap_or(0);
        let s = &stats;
        let r = std::sync::atomic::Ordering::Relaxed;
        info!(
            routing_nodes = routing,
            hashes_sampled = s.hashes_sampled.load(r),
            hashes_unique = s.hashes_unique.load(r),
            fetches_attempted = s.fetches_attempted.load(r),
            fetches_failed = s.fetches_failed.load(r),
            metadata_verified = s.metadata_verified.load(r),
            records_persisted = s.records_persisted.load(r),
            "crawl stats"
        );
        info!(
            connect_timeout = s.connect_timeout.load(r),
            connect_refused = s.connect_refused.load(r),
            no_bep10 = s.no_bep10.load(r),
            no_ut_metadata = s.no_ut_metadata.load(r),
            metadata_rejected = s.metadata_rejected.load(r),
            sha1_mismatch = s.sha1_mismatch.load(r),
            empty_peers = s.empty_peers.load(r),
            fetch_deadline = s.fetch_deadline.load(r),
            peer_errors_other = s.peer_errors_other.load(r),
            "peer failure breakdown"
        );
    }
}

async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
        _ = term.recv() => info!("received SIGTERM"),
    }
}

fn query(args: config::QueryArgs) -> Result<()> {
    let storage = Storage::open(&args.db)?;
    if args.failures {
        return query_failures(&storage);
    }
    let rows = storage.search(&args.name)?;
    if rows.is_empty() {
        println!("no matches for {:?}", args.name);
        return Ok(());
    }
    for r in rows {
        let category = match r.category {
            storage::Category::Movie => "movie",
            storage::Category::Tv => "tv",
            storage::Category::Other => "other",
        };
        let year = r.year.map_or("-".to_string(), |y| y.to_string());
        let size = r.size_bytes.map_or("-".to_string(), |b| {
            if b >= 1024 * 1024 * 1024 {
                format!("{:.1} GiB", b as f64 / (1024.0 * 1024.0 * 1024.0))
            } else if b >= 1024 * 1024 {
                format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0))
            } else {
                format!("{b} B")
            }
        });
        println!("{name}\t{category}\t{year}\t{size}", name = r.name);
    }
    Ok(())
}

fn query_failures(storage: &Storage) -> Result<()> {
    let rows = storage.failure_breakdown()?;
    if rows.is_empty() {
        println!("no failed fetches recorded yet");
        return Ok(());
    }
    println!("failed fetches by dominant reason:");
    for (reason, count) in rows {
        println!("  {count:>8}  {reason}");
    }
    Ok(())
}

/// Delete the database (and its WAL/SHM sidecars) and the routing state
/// directory so a subsequent `run` starts from scratch.
fn purge(args: &config::PurgeArgs) -> Result<()> {
    let db = std::path::Path::new(&args.db);
    let targets = [
        args.db.clone(),
        format!("{}-wal", args.db),
        format!("{}-shm", args.db),
    ];

    println!("Purging crawl data:");
    let mut removed = Vec::new();
    for t in &targets {
        let p = std::path::Path::new(t);
        if p.exists() {
            removed.push(t.clone());
        }
    }
    if args.state_dir.exists() {
        removed.push(args.state_dir.display().to_string());
    }

    if removed.is_empty() {
        println!("  nothing to purge");
        return Ok(());
    }
    for r in &removed {
        println!("  - {r}");
    }

    if !args.yes {
        eprint!("Delete these files and the routing state? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    for t in &targets {
        let p = std::path::Path::new(t);
        match p.metadata() {
            Ok(_) => {
                std::fs::remove_file(p).with_context(|| format!("remove {}", p.display()))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", p.display())),
        }
    }
    if args.state_dir.exists() {
        std::fs::remove_dir_all(&args.state_dir)
            .with_context(|| format!("remove {}", args.state_dir.display()))?;
    }

    println!("purged");
    let _ = db;
    Ok(())
}
