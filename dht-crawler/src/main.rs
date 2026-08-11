mod config;
mod dht;
mod filter;
mod metadata;
mod net;
mod stats;
mod storage;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
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
        concurrency = args.concurrency,
        "dht crawler starting"
    );

    let (hash_tx, hash_rx) = tokio::sync::mpsc::channel(SAMPLER_CHANNEL);
    let (record_tx, record_rx) = tokio::sync::mpsc::channel(RECORD_CHANNEL);

    let sampler_cfg = dht::SamplerConfig {
        queries_per_second: args.sampler_qps,
        concurrency: args.sampler_loops,
        min_seen: args.min_seen,
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
            concurrency: args.concurrency,
            lookup_concurrency: args.lookup_concurrency,
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

/// Single-threaded storage writer: batches records into transactions.
async fn write_loop(
    mut rx: tokio::sync::mpsc::Receiver<TorrentRecord>,
    storage: Storage,
    stats: Arc<CrawlStats>,
) {
    const BATCH: usize = 256;
    let mut batch: Vec<TorrentRecord> = Vec::with_capacity(BATCH);
    while let Some(record) = rx.recv().await {
        batch.push(record);
        if batch.len() >= BATCH {
            if let Err(e) = storage.insert_batch(&batch) {
                error!(error = %e, "storage batch failed");
            }
            batch.clear();
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
        info!(
            routing_nodes = routing,
            hashes_sampled = s.hashes_sampled.load(std::sync::atomic::Ordering::Relaxed),
            hashes_unique = s.hashes_unique.load(std::sync::atomic::Ordering::Relaxed),
            fetches_attempted = s.fetches_attempted.load(std::sync::atomic::Ordering::Relaxed),
            fetches_failed = s.fetches_failed.load(std::sync::atomic::Ordering::Relaxed),
            metadata_verified = s.metadata_verified.load(std::sync::atomic::Ordering::Relaxed),
            records_persisted = s.records_persisted.load(std::sync::atomic::Ordering::Relaxed),
            "crawl stats"
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
    let rows = storage.search(&args.name)?;
    if rows.is_empty() {
        println!("no matches for {:?}", args.name);
        return Ok(());
    }
    for r in rows {
        let category = match r.category {
            storage::Category::Movie => "movie",
            storage::Category::Tv => "tv",
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
