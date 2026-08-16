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

/// Read current jemalloc heap stats (MB). `None` when the allocator isn't
/// jemalloc or the ctl API is unavailable; the stats loop logs 0 then.
fn jemalloc_allocator_stats() -> (u64, u64, u64, u64) {
    use jemalloc_ctl::{epoch, stats};
    let _ = epoch::advance();
    let mb = |v: Option<usize>| v.map_or(0, |v| (v / (1024 * 1024)) as u64);
    (
        mb(stats::allocated::read().ok()),
        mb(stats::active::read().ok()),
        mb(stats::mapped::read().ok()),
        mb(stats::retained::read().ok()),
    )
}

/// Trigger a jemalloc heap profile dump (`prof.dump` mallctl). Requires the
/// process to have been started with `MALLOC_CONF=prof:true` (and a
/// `prof_prefix`), otherwise this is a no-op. Each dump appends a suffixed
/// file under the configured prefix; diff two dumps with `jeprof`.
fn jemalloc_prof_dump() {
    use std::os::raw::{c_char, c_void};
    unsafe {
        // mallctl("prof.dump", NULL, NULL, NULL, 0) — null value triggers a
        // dump to the configured prof_prefix with an auto-suffixed filename.
        let name = c"prof.dump".as_ptr() as *const c_char;
        let r = jemalloc_sys::mallctl(
            name,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
        if r != 0 {
            tracing::warn!(ret = r, "jemalloc prof.dump failed");
        } else {
            tracing::info!("jemalloc prof.dump written");
        }
    }
}

const SAMPLER_CHANNEL: usize = 8192;
const RECORD_CHANNEL: usize = 4096;
const STATS_INTERVAL: Duration = Duration::from_secs(30);
/// Per-hash in-flight-fetch budget permitted before aborting at shutdown.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// Run the crawl daemon until SIGTERM/SIGINT: N BEP 51 samplers → metadata
/// fetcher → storage writer, then drain and persist state on shutdown.
pub async fn run(args: RunArgs) -> Result<()> {
    // Unix seconds at process start, persisted with every stats snapshot so the
    // admin API/dashboard can reset cumulative-rate windows on restart (the
    // in-process counters reset to zero at process start).
    let process_start_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let storage = Storage::connect(&args.pg).await?;
    let stats = Arc::new(CrawlStats::default());
    let blocklist = Arc::new(Blocklist::load(args.blocklist.as_deref())?);
    let shared = crate::redis::init_shared(args.redis_url.clone(), Some(args.redis_prefix.clone())).await;

    let instances = args.instances.max(1);
    info!(
        port = args.port,
        instances = instances,
        ipv6 = args.ipv6,
        pg = %args.pg,
        concurrency = args.effective_concurrency(),
        aggressive = args.aggressive,
        "dht crawler starting"
    );

    // Each instance gets its own DHT node/sampler but shares one storage, one
    // fetch pool, and one Redis node pool. There is no file persistence: node
    // IDs live in Redis, and every instance seeds from the shared pool so all
    // converge on one table (DRY).
    let pool = shared.node_pool().await;
    info!(pool_size = pool.len(), "seeding instances from shared node pool");

    let mut handles = Vec::with_capacity(instances);
    for i in 0..instances {
        let seeds = pool.clone();
        let handle = discovery::start_dht(&args, i, seeds, &shared).await?;
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
            discovery::grow_routing(handle, Duration::from_millis(250), shutdown).await;
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
        min_sightings: args.min_sightings,
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
            sweep_stats.liveness_sweeps.fetch_add(1, rel);
            tracing::debug!(
                sweep = sweep_stats.liveness_sweeps.load(rel),
                evicted = evicted.len(),
                entries = sweep_liveness.len(),
                "liveness sweep"
            );
            // Sample-log a fraction of filtered hashes so the shadow run can
            // inspect whether they look like dead garbage or plausible-live
            // torrents (hex hash + max distinct sources reached).
            let mut sample_logged = 0usize;
            for ev in evicted {
                if sweep_shadow.is_none() {
                    continue;
                }
                match ev.max_sources {
                    1 => {
                        sweep_stats.shadow_near_miss_1.fetch_add(1, rel);
                        // Discriminate: did the sole source refresh (sparsity)
                        // or report exactly once (backoff-stalled)?
                        if ev.sightings > 1 {
                            sweep_stats.shadow_near_miss_1_sparse.fetch_add(1, rel);
                        } else {
                            sweep_stats.shadow_near_miss_1_stalled.fetch_add(1, rel);
                        }
                    }
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
    // sender owned by this scope until they finish. The retry worker also gets
    // a sender so it can re-queue failed hashes.
    let hash_tx_retry = hash_tx.clone();
    drop(hash_tx);

    let primary = handles[0].clone();
    // Shared in-flight set so the retry worker and the fetcher never fetch the
    // same hash concurrently.
    let in_flight: Arc<std::sync::Mutex<std::collections::HashSet<gaia_core::Id20>>> =
        Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
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
        in_flight.clone(),
    ));

    let writer = tokio::spawn(write_loop(record_rx, storage.clone(), stats.clone()));

    // Active retry worker: drains retry-eligible failed hashes into the same
    // fetch queue with its own bounded concurrency so it can't starve fresh
    // fetches. It shares the in_flight set to avoid double-fetching.
    let retry_semaphore = Arc::new(tokio::sync::Semaphore::new(64));
    let retry_task = tokio::spawn(crate::retry::run_retry_worker(
        storage.clone(),
        hash_tx_retry,
        retry_semaphore,
        in_flight,
        stats.clone(),
        args.max_attempts,
        shutdown.clone(),
    ));

    let stats_task = tokio::spawn(stats_loop(
        handles.clone(),
        stats.clone(),
        liveness.clone(),
        storage.clone(),
        process_start_ts,
    ));

    // Optional built-in HTTP health endpoint (--health-port, default off). The
    // DHT UDP path has no HTTP surface; this gives the container healthcheck and
    // external monitoring a real endpoint plus a Postgres liveness probe.
    if args.health_port > 0 {
        tokio::spawn(crate::health::serve(
            args.health_port,
            process_start_ts,
            storage.clone(),
            shutdown.clone(),
        ));
    }

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
    retry_task.abort();
    // Dropping the fetcher aborts its JoinSet; the fetch tasks' `record_tx`
    // clones then drop, closing the write channel so the writer drains.
    let _ = writer.await;
    stats_task.abort();

    // Persist the shared node pool (union of every instance's routing table)
    // to Redis, then shut each instance down cleanly. No files are written.
    let mut pool_nodes: Vec<String> = Vec::new();
    for handle in &handles {
        for (_, addr) in handle.get_routing_nodes().await {
            pool_nodes.push(addr.to_string());
        }
        if let Err(e) = handle.shutdown_and_wait().await {
            error!(error = %e, "failed to shut down DHT instance");
        }
    }
    shared.node_pool_put(&pool_nodes).await;
    info!(pool_size = pool_nodes.len(), "persisted shared node pool to redis");
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
                            if let Err(e) = storage.insert_batch(&batch).await {
                                error!(error = %e, "storage batch failed");
                            }
                            batch.clear();
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(FLUSH_INTERVAL), if !batch.is_empty() => {
                if let Err(e) = storage.insert_batch(&batch).await {
                    error!(error = %e, "storage batch failed");
                }
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        if let Err(e) = storage.insert_batch(&batch).await {
            error!(error = %e, "storage final batch failed");
        }
    }
    let _ = stats;
}

async fn stats_loop(
    handles: Vec<DhtHandle>,
    stats: Arc<CrawlStats>,
    liveness: Arc<discovery::LivenessCounter>,
    storage: Storage,
    process_start_ts: u64,
) {
    let mut tick = tokio::time::interval(STATS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_unique: u64 = 0;
    // System resource sampling for the monitoring snapshot (tun0 = tunnel iface).
    let mut sysmetrics = crate::sysmetrics::SysMetricSampler::new("tun0", std::path::Path::new("/data"));
    // Heap profiling: when GAIA_PROF_DUMP_EVERY_TICKS is set, dump a jemalloc
    // heap profile every N ticks (requires MALLOC_CONF=prof:true,prof_prefix).
    let prof_every = std::env::var("GAIA_PROF_DUMP_EVERY_TICKS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let mut tick_count: u32 = 0;
    loop {
        tick.tick().await;
        tick_count = tick_count.wrapping_add(1);
        if let Some(every) = prof_every {
            if every > 0 && tick_count.is_multiple_of(every) {
                jemalloc_prof_dump();
            }
        }
        // Per-instance + fleet-wide aggregate routing/actor stats. The
        // per-instance breakdown identifies redundant instances; the aggregate
        // is what operators want to watch.
        let mut routing = 0usize;
        let mut announced = 0usize;
        let mut active_lookups = 0usize;
        let mut announce_tokens = 0usize;
        let mut pending_queries = 0usize;
        let mut announces_received = 0u64;
        let mut announces_token_rejected = 0u64;
        let mut announces_suppressed_readonly = 0u64;
        let mut lookups_received = 0u64;
        let mut per_instance = Vec::with_capacity(handles.len());
        for h in &handles {
            let n = h.node_count().await.unwrap_or(0);
            routing += n;
            let st = h.stats().await;
            let total = st.as_ref().map(|s| s.total_queries_sent).unwrap_or(0);
            per_instance.push(format!("{n}/{}q", total));
            if let Ok(s) = st {
                announced += s.peer_store_info_hashes;
                active_lookups += s.active_lookups;
                announce_tokens += s.announce_tokens;
                pending_queries += s.pending_queries;
                announces_received += s.announces_received;
                announces_token_rejected += s.announces_token_rejected;
                announces_suppressed_readonly += s.announces_suppressed_readonly;
                lookups_received += s.lookups_received;
            }
        }
        let s = &stats;
        let r = std::sync::atomic::Ordering::Relaxed;
        // Unique-hash discovery rate over the last tick (unique/hr) so the
        // node-diversity and keyspace-sweep levers are visible independently of
        // fetch success.
        let unique_now = s.hashes_unique.load(r);
        let unique_delta = unique_now.saturating_sub(last_unique);
        let unique_per_hr = unique_delta as f64 / STATS_INTERVAL.as_secs_f64() * 3600.0;
        last_unique = unique_now;
        // Allocator state (jemalloc): allocated = live heap bytes; active =
        // committed-but-used pages; mapped = address space; retained = pages
        // kept for reuse but not committed (RSS is roughly active+retained).
        // A real leak grows `allocated`; page-retention churn grows only
        // `mapped`/`retained`. Diagnoses whether RSS creep is a true leak or
        // allocator behavior.
        let (jemalloc_allocated, jemalloc_active, jemalloc_mapped, jemalloc_retained) =
            jemalloc_allocator_stats();
        info!(
            jemalloc_allocated = jemalloc_allocated,
            jemalloc_active = jemalloc_active,
            jemalloc_mapped = jemalloc_mapped,
            jemalloc_retained = jemalloc_retained,
            routing_nodes = routing,
            instance_nodes = per_instance.join(","),
            announced_hashes = announced,
            active_lookups = active_lookups,
            announce_tokens = announce_tokens,
            pending_queries = pending_queries,
            announces_received = announces_received,
            announces_token_rejected = announces_token_rejected,
            announces_suppressed_readonly = announces_suppressed_readonly,
            lookups_received = lookups_received,
            announces_deduped_redis = s.announces_deduped_redis.load(r),
            announces_emitted = s.announces_emitted.load(r),
            hashes_sampled = s.hashes_sampled.load(r),
            hashes_unique = unique_now,
            unique_per_hr = format!("{unique_per_hr:.1}"),
            hashes_announced = s.hashes_announced.load(r),
            shadow_emitted = s.shadow_emitted.load(r),
            shadow_filtered = s.shadow_filtered.load(r),
            shadow_near_miss_1 = s.shadow_near_miss_1.load(r),
            shadow_near_miss_1_sparse = s.shadow_near_miss_1_sparse.load(r),
            shadow_near_miss_1_stalled = s.shadow_near_miss_1_stalled.load(r),
            shadow_near_miss_2 = s.shadow_near_miss_2.load(r),
            liveness_entries = liveness.len(),
            liveness_sweeps = s.liveness_sweeps.load(r),
            fetches_attempted = s.fetches_attempted.load(r),
            fetches_failed = s.fetches_failed.load(r),
            fetch_in_flight = s.fetch_in_flight.load(r),
            queue_depth = s.queue_depth.load(r),
            metadata_verified = s.metadata_verified.load(r),
            verified_announced = s.verified_announced.load(r),
            verified_sampled = s.verified_sampled.load(r),
            verified_lookedup = s.verified_lookedup.load(r),
            verified_tracker = s.verified_tracker.load(r),
            scrape_saw_seeds = s.scrape_saw_seeds.load(r),
            verified_with_seeds = s.verified_with_seeds.load(r),
            verified_without_seeds = s.verified_without_seeds.load(r),
            failed_with_seeds = s.failed_with_seeds.load(r),
            failed_without_seeds = s.failed_without_seeds.load(r),
            tracker_resolved = s.tracker_resolved.load(r),
            lookups_emitted = s.lookups_emitted.load(r),
            lookups_deduped_redis = s.lookups_deduped_redis.load(r),
            discriminator_filtered = s.discriminator_filtered.load(r),
            terminal_dead = s.terminal_dead.load(r),
            records_persisted = s.records_persisted.load(r),
            "crawl stats"
        );
        info!(
            connect_timeout = s.connect_timeout.load(r),
            connect_refused = s.connect_refused.load(r),
            connection_reset = s.connection_reset.load(r),
            connection_closed = s.connection_closed.load(r),
            no_bep10 = s.no_bep10.load(r),
            no_ut_metadata = s.no_ut_metadata.load(r),
            metadata_rejected = s.metadata_rejected.load(r),
            parse_error = s.parse_error.load(r),
            dht_lookup_failed = s.dht_lookup_failed.load(r),
            lookup_pool_exhausted = s.lookup_pool_exhausted.load(r),
            sha1_mismatch = s.sha1_mismatch.load(r),
            empty_peers = s.empty_peers.load(r),
            fetch_deadline = s.fetch_deadline.load(r),
            early_abort = s.early_abort.load(r),
            peer_errors_other = s.peer_errors_other.load(r),
            "peer failure breakdown"
        );

        // Persist the full monitoring snapshot to Postgres. Best-effort: a
        // failed write is logged and the crawl continues.
        let sys = sysmetrics.sample();
        let snap = crate::stats::CrawlSnapshot {
            process_start_ts,
            hashes_sampled: s.hashes_sampled.load(r),
            hashes_unique: s.hashes_unique.load(r),
            hashes_announced: s.hashes_announced.load(r),
            announces_deduped_redis: s.announces_deduped_redis.load(r),
            announces_emitted: s.announces_emitted.load(r),
            shadow_emitted: s.shadow_emitted.load(r),
            shadow_filtered: s.shadow_filtered.load(r),
            shadow_near_miss_1: s.shadow_near_miss_1.load(r),
            shadow_near_miss_2: s.shadow_near_miss_2.load(r),
            shadow_near_miss_1_sparse: s.shadow_near_miss_1_sparse.load(r),
            shadow_near_miss_1_stalled: s.shadow_near_miss_1_stalled.load(r),
            liveness_sweeps: s.liveness_sweeps.load(r),
            fetches_attempted: s.fetches_attempted.load(r),
            fetches_failed: s.fetches_failed.load(r),
            metadata_verified: s.metadata_verified.load(r),
            records_persisted: s.records_persisted.load(r),
            terminal_dead: s.terminal_dead.load(r),
            fetch_in_flight: s.fetch_in_flight.load(r),
            queue_depth: s.queue_depth.load(r),
            connect_timeout: s.connect_timeout.load(r),
            connect_refused: s.connect_refused.load(r),
            connection_reset: s.connection_reset.load(r),
            connection_closed: s.connection_closed.load(r),
            no_bep10: s.no_bep10.load(r),
            no_ut_metadata: s.no_ut_metadata.load(r),
            metadata_rejected: s.metadata_rejected.load(r),
            parse_error: s.parse_error.load(r),
            dht_lookup_failed: s.dht_lookup_failed.load(r),
            lookup_pool_exhausted: s.lookup_pool_exhausted.load(r),
            sha1_mismatch: s.sha1_mismatch.load(r),
            empty_peers: s.empty_peers.load(r),
            fetch_deadline: s.fetch_deadline.load(r),
            early_abort: s.early_abort.load(r),
            peer_errors_other: s.peer_errors_other.load(r),
            verified_announced: s.verified_announced.load(r),
            verified_sampled: s.verified_sampled.load(r),
            verified_retried: s.verified_retried.load(r),
            retry_worker_scans: s.retry_worker_scans.load(r),
            verified_lookedup: s.verified_lookedup.load(r),
            verified_tracker: s.verified_tracker.load(r),
            scrape_saw_seeds: s.scrape_saw_seeds.load(r),
            verified_with_seeds: s.verified_with_seeds.load(r),
            verified_without_seeds: s.verified_without_seeds.load(r),
            failed_with_seeds: s.failed_with_seeds.load(r),
            failed_without_seeds: s.failed_without_seeds.load(r),
            discriminator_filtered: s.discriminator_filtered.load(r),
            lookups_emitted: s.lookups_emitted.load(r),
            lookups_deduped_redis: s.lookups_deduped_redis.load(r),
            routing_nodes: routing as u64,
            announced_hashes: announced as u64,
            active_lookups: active_lookups as u64,
            announce_tokens: announce_tokens as u64,
            pending_queries: pending_queries as u64,
            announces_received,
            announces_token_rejected,
            announces_suppressed_readonly,
            lookups_received,
            unique_per_hr,
            jemalloc_allocated: jemalloc_allocated as f64,
            jemalloc_active: jemalloc_active as f64,
            jemalloc_mapped: jemalloc_mapped as f64,
            jemalloc_retained: jemalloc_retained as f64,
            net_rx_bytes: sys.net_rx_bytes,
            net_tx_bytes: sys.net_tx_bytes,
            net_rx_rate_bps: sys.net_rx_rate_bps,
            net_tx_rate_bps: sys.net_tx_rate_bps,
            host_mem_total: sys.host_mem_total,
            host_mem_available: sys.host_mem_available,
            container_mem_current: sys.container_mem_current,
            cpu_percent: sys.cpu_percent,
            disk_total_bytes: sys.disk_total_bytes,
            disk_free_bytes: sys.disk_free_bytes,
            loadavg_1: sys.loadavg_1,
            loadavg_5: sys.loadavg_5,
            loadavg_15: sys.loadavg_15,
        };
        storage
            .record_crawl_stats(&snap, &serde_json::json!({ "instances": per_instance }))
            .await;
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
