use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use irontide_dht::DhtHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::cli::RunArgs;
use crate::discovery::{self, Sampler, SamplerConfig};
use crate::fetch::{run_fetcher, FetcherConfig};
use crate::net::Blocklist;
use crate::stats::CrawlStats;
use crate::storage::{Storage, TorrentRecord};

const SAMPLER_CHANNEL: usize = 8192;
const RECORD_CHANNEL: usize = 4096;
const STATS_INTERVAL: Duration = Duration::from_secs(30);
/// Per-hash in-flight-fetch budget permitted before aborting at shutdown.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// Run the crawl daemon until SIGTERM/SIGINT: BEP 51 sampler → metadata
/// fetcher → storage writer, then drain and persist state on shutdown.
pub async fn run(args: RunArgs) -> Result<()> {
    let state_dir = args.state_dir.clone();
    if !state_dir.exists() {
        std::fs::create_dir_all(&state_dir)?;
    }

    let storage = Storage::open(&args.db)?;
    let stats = Arc::new(CrawlStats::default());
    let blocklist = Arc::new(Blocklist::load(args.blocklist.as_deref())?);

    let handle = discovery::start_dht(&args, args.state_dir.clone()).await?;
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

    let sampler_cfg = SamplerConfig {
        queries_per_second: args.effective_sampler_qps(),
        concurrency: args.effective_sampler_loops(),
        min_seen: args.effective_min_seen(),
        max_interval_secs: args.sampler_max_interval,
    };
    let shutdown = CancellationToken::new();
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
        // Passive announcement intake: hashes other nodes announced to us that
        // are sitting in the actor's internal peer_store. If this stays near 0,
        // announcement capture is not worth patching irontide for; if it grows
        // large, a `peer_store_hashes()` reader becomes a valuable second
        // discovery source alongside BEP 51 sampling.
        let announced = handle
            .stats()
            .await
            .map(|s| s.peer_store_info_hashes)
            .unwrap_or(0);
        let s = &stats;
        let r = std::sync::atomic::Ordering::Relaxed;
        info!(
            routing_nodes = routing,
            announced_hashes = announced,
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
