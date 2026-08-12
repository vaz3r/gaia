use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use gaia_dht::DhtHandle;
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

/// Run the crawl daemon until SIGTERM/SIGINT: N BEP 51 samplers → metadata
/// fetcher → storage writer, then drain and persist state on shutdown.
pub async fn run(args: RunArgs) -> Result<()> {
    let state_dir = args.state_dir.clone();
    std::fs::create_dir_all(&state_dir)?;

    let storage = Storage::open(&args.db)?;
    let stats = Arc::new(CrawlStats::default());
    let blocklist = Arc::new(Blocklist::load(args.blocklist.as_deref())?);
    let shared = crate::redis::init_shared(args.redis_url.clone()).await;

    let instances = args.instances.max(1);
    info!(
        port = args.port,
        instances = instances,
        ipv6 = args.ipv6,
        db = %args.db,
        state_dir = %state_dir.display(),
        concurrency = args.effective_concurrency(),
        aggressive = args.aggressive,
        "dht crawler starting"
    );

    // Each instance gets its own DHT node/sampler but shares one storage and
    // one fetch pool. Instance 0 uses the configured state dir (may already
    // hold a warm routing table); instances 1..N bootstrap from instance 0's
    // known-live nodes so they do not start from an empty table.
    let mut handles = Vec::with_capacity(instances);
    for i in 0..instances {
        let instance_dir = if instances == 1 {
            args.state_dir.clone()
        } else {
            state_dir.join(format!("instance-{i}"))
        };
        std::fs::create_dir_all(&instance_dir)?;
        // Instance 0 loads the configured state; later instances reuse its
        // persisted nodes as bootstrap seeds (captured before instance 0's
        // in-memory table is used).
        let seeds = if i == 0 {
            Vec::new()
        } else {
            discovery::seed_nodes_from_state(&args.state_dir)
        };
        let handle = discovery::start_dht(&args, instance_dir, i, seeds).await?;
        handles.push(handle);
    }

    let shutdown = CancellationToken::new();

    // Spawn one continuous routing grower per instance so each routing table
    // climbs toward --max-nodes throughout the crawl (not just at startup).
    // 100ms keeps the table filling toward the 4096-node target; the node pool
    // is the binding constraint on unique discovery, so we spend more of the
    // DHT budget here than the efficient phase did (which throttled to 1s).
    let mut growers = tokio::task::JoinSet::new();
    for handle in &handles {
        let handle = handle.clone();
        let shutdown = shutdown.clone();
        growers.spawn(async move {
            discovery::grow_routing(handle, Duration::from_millis(100), shutdown).await;
        });
    }

    let (hash_tx, hash_rx) =
        tokio::sync::mpsc::channel(SAMPLER_CHANNEL.saturating_mul(args.scale.max(1)));
    let (record_tx, record_rx) =
        tokio::sync::mpsc::channel(RECORD_CHANNEL.saturating_mul(args.scale.max(1)));

    let sampler_cfg = SamplerConfig {
        queries_per_second: args.effective_sampler_qps(),
        concurrency: args.effective_sampler_loops(),
        min_seen: args.effective_min_seen(),
        min_seen_shadow: (args.min_seen_shadow > 0).then_some(args.min_seen_shadow),
        max_interval_secs: args.sampler_max_interval,
    };

    // All instances share the hash channel; the fetcher consumes from it once.
    // One in-memory bloom filter caches known-blocked hashes so repeated
    // re-sampling of dead hashes stops hitting the database after the first
    // authoritative check per hash.
    let seen_bloom = crate::bloom::SharedBloom::new(27, 7);
    // One shared liveness counter across all instances' sampler loops: a hash
    // is fetched only after N distinct DHT nodes corroborate it within the
    // rolling window (the liveness gate), with shadow-mode observation.
    let liveness = discovery::LivenessCounter::new(discovery::LivenessConfig {
        window: Duration::from_secs(args.liveness_window_secs),
        cap: args.liveness_cap,
        max_entries: args.liveness_max_entries,
    });
    // Periodic sweep: expire one-hit-wonders and enforce the global backstop,
    // feeding shadow near-miss counters.
    let sweep_liveness = liveness.clone();
    let sweep_stats = stats.clone();
    let sweep_shutdown = shutdown.clone();
    let sweep_shadow = (args.min_seen_shadow > 0).then_some(args.min_seen_shadow);
    let mut sweep = tokio::task::JoinSet::new();
    sweep.spawn(async move {
        loop {
            tokio::select! {
                _ = sweep_shutdown.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
            let evicted = sweep_liveness.sweep(Instant::now());
            let rel = std::sync::atomic::Ordering::Relaxed;
            // Sample-log a fraction of filtered hashes so the shadow run can
            // inspect whether they look like dead garbage or plausible-live
            // torrents (hex hash + max distinct sources reached).
            let mut sample_logged = 0usize;
            for ev in evicted {
                if sweep_shadow.is_none() {
                    continue;
                }
                match ev.max_sources {
                    1 => { sweep_stats.shadow_near_miss_1.fetch_add(1, rel); }
                    2 => { sweep_stats.shadow_near_miss_2.fetch_add(1, rel); }
                    _ => {}
                }
                // Everything expired without reaching the shadow threshold is
                // "would be filtered" under `--min-seen-shadow`.
                sweep_stats.shadow_filtered.fetch_add(1, rel);
                // Log ~1 in 1000 filtered hashes for qualitative inspection.
                let n = sweep_stats.shadow_filtered.load(rel);
                if n % 1000 == 1 && sample_logged < 3 {
                    sample_logged += 1;
                    let hex = ev
                        .hash
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    tracing::debug!(
                        shadow_filtered_hash = %hex,
                        max_sources = ev.max_sources,
                        "shadow: would filter under --min-seen-shadow"
                    );
                }
            }
        }
    });

    let mut samplers = tokio::task::JoinSet::new();
    for handle in &handles {
        let sampler = Sampler::new(
            handle.clone(),
            hash_tx.clone(),
            storage.clone(),
            &sampler_cfg,
            stats.clone(),
            shutdown.clone(),
            shared.clone(),
            seen_bloom.clone(),
            liveness.clone(),
        );
        samplers.spawn(async move { sampler.run().await });
    }

    // Passive intake: one subscriber per instance drains inbound announce_peer
    // events into the fetch pipeline with a live-peer dial hint. These hashes
    // are live by construction and skip get_peers discovery — the core of the
    // passive-intake architecture.
    let mut intake = tokio::task::JoinSet::new();
    for handle in &handles {
        let handle = handle.clone();
        let intake_tx = hash_tx.clone();
        let intake_stats = stats.clone();
        let intake_shared = shared.clone();
        let intake_shutdown = shutdown.clone();
        intake.spawn(async move {
            discovery::run_passive_intake(
                handle,
                intake_tx,
                intake_stats,
                intake_shared,
                intake_shutdown,
            )
            .await;
        });
    }

    // The shared channel must outlive all sampler + intake clones: keep a live
    // sender owned by this scope until they finish.
    drop(hash_tx);

    let primary = handles[0].clone();
    let mut fetcher = tokio::spawn(run_fetcher(
        hash_rx,
        record_tx,
        primary.clone(),
        storage.clone(),
        FetcherConfig {
            concurrency: args.effective_concurrency(),
            lookup_concurrency: args.effective_lookup_concurrency(),
            blocklist,
            shared: shared.clone(),
        },
        stats.clone(),
    ));

    let writer = tokio::spawn(write_loop(record_rx, storage.clone(), stats.clone()));

    let stats_task = tokio::spawn(stats_loop(handles.clone(), stats.clone()));

    wait_for_shutdown().await;

    info!("shutdown signal received, draining pipeline");

    // Cancel the sampler + intake loops so they drop their `emit` clones and
    // close the fetch channel; the fetcher then drains its in-flight work.
    shutdown.cancel();
    while growers.join_next().await.is_some() {}
    while samplers.join_next().await.is_some() {}
    while intake.join_next().await.is_some() {}
    while sweep.join_next().await.is_some() {}
    let _ = tokio::time::timeout(SHUTDOWN_DRAIN + Duration::from_secs(5), &mut fetcher).await;
    fetcher.abort();
    // Dropping the fetcher aborts its JoinSet; the fetch tasks' `record_tx`
    // clones then drop, closing the write channel so the writer drains.
    let _ = writer.await;
    stats_task.abort();

    // Persist every instance's routing table before exit.
    for handle in &handles {
        if let Err(e) = handle.shutdown_and_wait().await {
            error!(error = %e, "failed to persist routing table");
        }
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

async fn stats_loop(handles: Vec<DhtHandle>, stats: Arc<CrawlStats>) {
    let mut tick = tokio::time::interval(STATS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_unique: u64 = 0;
    loop {
        tick.tick().await;
        let primary = &handles[0];
        let routing = primary.node_count().await.unwrap_or(0);
        // Per-instance routing node counts so a redundant instance (one that
        // burns tunnel bandwidth without contributing nodes) is identifiable.
        let mut per_instance = Vec::with_capacity(handles.len());
        for h in &handles {
            let n = h.node_count().await.unwrap_or(0);
            let total = h.stats().await.map(|s| s.total_queries_sent).unwrap_or(0);
            per_instance.push(format!("{n}/{}q", total));
        }
        // Passive announcement intake: hashes other nodes announced to us that
        // are sitting in the actor's internal peer_store. Measured in the
        // node-diversity phase: a peer_store drain would require patching
        // irontide, and the announced-hash yield (~1.9% of unique) didn't
        // justify it — so this stays a diagnostic counter only.
        let announced = primary
            .stats()
            .await
            .map(|s| s.peer_store_info_hashes)
            .unwrap_or(0);
        let s = &stats;
        let r = std::sync::atomic::Ordering::Relaxed;
        // Unique-hash discovery rate over the last tick (unique/hr) so the
        // node-diversity and keyspace-sweep levers are visible independently of
        // fetch success.
        let unique_now = s.hashes_unique.load(r);
        let unique_delta = unique_now.saturating_sub(last_unique);
        let unique_per_hr = unique_delta as f64 / STATS_INTERVAL.as_secs_f64() * 3600.0;
        last_unique = unique_now;
        info!(
            routing_nodes = routing,
            instance_nodes = per_instance.join(","),
            announced_hashes = announced,
            hashes_sampled = s.hashes_sampled.load(r),
            hashes_unique = unique_now,
            unique_per_hr = format!("{unique_per_hr:.1}"),
            hashes_announced = s.hashes_announced.load(r),
            shadow_emitted = s.shadow_emitted.load(r),
            shadow_filtered = s.shadow_filtered.load(r),
            shadow_near_miss_1 = s.shadow_near_miss_1.load(r),
            shadow_near_miss_2 = s.shadow_near_miss_2.load(r),
            fetches_attempted = s.fetches_attempted.load(r),
            fetches_failed = s.fetches_failed.load(r),
            fetch_in_flight = s.fetch_in_flight.load(r),
            queue_depth = s.queue_depth.load(r),
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
            early_abort = s.early_abort.load(r),
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
