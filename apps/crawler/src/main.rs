mod config;
mod dht;
mod harvest;
mod krpc;
mod logging;
mod metrics;
mod net;
mod router;
mod storage;
mod trace;
mod verify;

use crate::config::Config;
use crate::dht::routing_table::{NodeInfo, RoutingTable};
use crate::dht::walker::Walker;
use crate::harvest::{HarvestEvent, Harvester, Source};
use crate::krpc::NodeId;
use crate::krpc::token::TokenGenerator;
use crate::krpc::tx_state::TxTable;
use crate::metrics::Metrics;
use crate::net::rate_limit::RateLimiter;
use crate::router::Router;
use crate::storage::batch_writer::BatchWriter;
use crate::storage::janitor::JanitorConfig;
use crate::storage::jobs::{RetryConfig as JobRetryConfig, VerifyStore};
use crate::storage::pg::PoolConfig;
use crate::storage::sightings::SightingWriter;
use crate::trace::TraceConfig;
use crate::verify::fetch_pool::FetchParams;
use crate::verify::peer_cache::PeerCache;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc};

#[tokio::main]
async fn main() {
    let config = Config::load();

    if let Err(e) = config.validate() {
        eprintln!("invalid configuration: {e}");
        std::process::exit(1);
    }

    let log_dropped = Arc::new(AtomicU64::new(0));
    let _logging_guard = logging::init(&config, log_dropped.clone());

    log_effective_config(&config);

    tracing::info!(
        git_hash = env!("CRAW_GIT_HASH"),
        target_arch = env!("CRAW_TARGET_ARCH"),
        target_os = env!("CRAW_TARGET_OS"),
        "crawler starting"
    );

    std::fs::create_dir_all(&config.data_dir).expect("create data dir");

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable is required");
    let pool = storage::pg::connect(
        &database_url,
        &PoolConfig {
            max_connections: config.storage.pg_pool_max_connections,
            min_connections: config.storage.pg_pool_min_connections,
            acquire_timeout_secs: config.storage.pg_pool_acquire_timeout_secs,
        },
    )
    .await
    .expect("connect to postgres");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    sqlx::query(
        "INSERT INTO metrics (ts, metric_name, metric_value) VALUES (now(), '_session_start', 0)",
    )
    .execute(&pool)
    .await
    .expect("write session start marker");

    if std::env::args().any(|a| a == "--backfill") {
        storage::backfill::run(&pool, &config.data_dir)
            .await
            .expect("backfill");
        tracing::info!("backfill complete");
        return;
    }

    let metrics = Arc::new(Metrics::new(log_dropped));
    crate::trace::TRACE_CONFIG
        .set(TraceConfig::new(
            config.trace_sample_rate,
            config.debug_ih.clone(),
        ))
        .unwrap_or(());

    let channel_capacity = config.channel_capacity.max(1);
    let fresh_capacity = config.fetch.fresh_channel_capacity.max(1);
    let (discovery_tx, discovery_rx) = mpsc::channel(channel_capacity);
    let (fresh_verify_tx, fresh_verify_rx) = mpsc::channel(fresh_capacity);
    let (verify_tx, verify_rx) = mpsc::channel(channel_capacity);
    let (announce_tx, announce_rx) = mpsc::channel(channel_capacity);

    let harvest_channel_capacity = config.harvest.harvest_channel_capacity.max(1);
    let (harvest_tx, harvest_rx) = mpsc::channel(harvest_channel_capacity);
    let harvester = Harvester::new(
        config.harvest.bloom_capacity,
        config.harvest.bloom_fp_rate,
        config.harvest.announce_bloom_ratio,
        config.harvest.announce_bloom_min,
        discovery_tx,
        fresh_verify_tx.clone(),
        verify_tx.clone(),
        announce_tx,
        metrics.clone(),
    );
    tokio::spawn(crate::harvest::run_harvester(harvest_rx, harvester));

    let peer_cache = Arc::new(PeerCache::new(
        Duration::from_secs(config.cache.peer_cache_ttl_secs),
        config.cache.peer_cache_max_entries,
    ));
    let peer_cache_cleanup = peer_cache.clone();

    let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
        broadcast::channel(1);

    tracing::info!(
        nodes = config.nodes,
        port_base = config.port_base,
        workers_per_node = config.worker_threads,
        sybils_per_node = config.dht.sybil_count,
        fetch_limit = config.fetch.global_fetch_limit,
        race_peers = config.fetch.race_peers,
        "crawler starting"
    );

    let bootstrap = resolve_bootstrap(&config.bootstrap).await;
    let mut node_routers = Vec::with_capacity(config.nodes);
    for i in 0..config.nodes {
        spawn_node(
            &config,
            i,
            &bootstrap,
            metrics.clone(),
            harvest_tx.clone(),
            &mut node_routers,
        )
        .await;
        tracing::info!(node = i, "dht node started");
    }
    let node_routers: Arc<Vec<Arc<Router>>> = Arc::new(node_routers);

    let sightings = Arc::new(SightingWriter::new(
        pool.clone(),
        config.storage.sighting_chunk_size,
    ));
    let sightings_run = sightings.clone().run(Duration::from_millis(
        config.storage.sighting_flush_interval_ms,
    ));
    let sightings_flush = flush_sightings(sightings.clone(), discovery_rx);

    let retry_backoffs = config
        .retry
        .backoff_sequence_secs
        .iter()
        .map(|&s| Duration::from_secs(s))
        .collect::<Vec<_>>();

    let verify_store = Arc::new(VerifyStore::new(
        pool.clone(),
        JobRetryConfig {
            max_retries: config.retry.max_retries,
            backoffs: retry_backoffs.clone(),
            scheduler_claim_limit: config.retry.scheduler_claim_limit,
            scheduler_fresh_ratio: config.retry.scheduler_fresh_ratio,
            stale_verifying_timeout_secs: config.retry.stale_verifying_timeout_secs,
        },
        metrics.clone(),
    ));
    let retry_run = verify_store.clone().run_scheduler(
        verify_tx.clone(),
        Duration::from_secs(config.retry.scheduler_interval_secs),
    );

    let batch_writer = Arc::new(BatchWriter::new(
        pool.clone(),
        retry_backoffs.clone(),
        config.retry.max_retries,
        config.retry.no_peers_terminal_on_first,
        config.retry.no_metadata_max_retries,
        config.storage.batch_flush_chunk,
        config.storage.torrent_batch_chunk,
    ));

    let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
    let batch_run = batch_writer.clone().run(
        Duration::from_secs(config.storage.batch_flush_interval_secs),
        shutdown_tx.subscribe(),
    );

    let janitor_pool = pool.clone();
    let janitor_config = JanitorConfig {
        dead_retention_secs: config.storage.janitor_dead_retention_secs,
        verified_retention_secs: config.storage.janitor_verified_retention_secs,
        batch_size: config.storage.janitor_batch_size,
        batch_sleep_ms: config.storage.janitor_batch_sleep_ms,
    };
    tokio::spawn(async move {
        let report = storage::janitor::run(&janitor_pool, &janitor_config).await;
        if report.dead_deleted == 0 && report.verified_deleted == 0 {
            tracing::info!("janitor: nothing to clean (table drained)");
        }
        let mut tick =
            tokio::time::interval(Duration::from_secs(config.storage.janitor_interval_secs));
        loop {
            tick.tick().await;
            let report = storage::janitor::run(&janitor_pool, &janitor_config).await;
            let _ = sqlx::query(
                "INSERT INTO metrics (ts, metric_name, metric_value) \
                 VALUES (now(), 'janitor_deleted', $1) \
                 ON CONFLICT (ts, metric_name) DO UPDATE SET metric_value = EXCLUDED.metric_value",
            )
            .bind(report.dead_deleted + report.verified_deleted)
            .execute(&janitor_pool)
            .await;
        }
    });

    let metrics_writer = Arc::new(storage::metrics_writer::MetricsWriter::new(
        pool.clone(),
        metrics.clone(),
    ));
    let metrics_run = metrics_writer.clone().run(Duration::from_secs(
        config.storage.metrics_flush_interval_secs,
    ));

    let announce_peer_cache = Arc::new(crate::verify::AnnouncePeerCache::new(
        Duration::from_secs(config.cache.announce_cache_ttl_secs),
        config.cache.announce_cache_max_entries,
        config.cache.announce_cache_initial_capacity,
        config.cache.announce_cache_shards,
    ));
    let peer_outcomes = Arc::new(crate::storage::peer_outcomes::PeerOutcomeWriter::new(
        pool.clone(),
        config.storage.peer_outcomes_chunk_size,
    ));
    let peer_outcomes_run = peer_outcomes.clone().run(Duration::from_secs(
        config.storage.peer_outcomes_flush_interval_secs,
    ));

    let pipeline = verify::run_pipeline(
        verify_rx,
        fresh_verify_rx,
        announce_rx,
        node_routers,
        if config.fetch.utp_enabled {
            utp_socket().await
        } else {
            None
        },
        metrics.clone(),
        batch_writer.clone(),
        peer_cache.clone(),
        announce_peer_cache,
        peer_outcomes,
        Arc::new(verify::ConnLimiter::new(
            config.fetch.max_connections_per_ip,
        )),
        verify::VerifyConfig {
            pipeline_limit: config.fetch.pipeline_limit,
            fetch_limit: config.fetch.global_fetch_limit,
            params: FetchParams {
                tcp_timeout: Duration::from_secs(config.fetch.tcp_timeout_secs),
                utp_timeout: Duration::from_secs(config.fetch.utp_timeout_secs),
                metadata_timeout: Duration::from_secs(config.fetch.metadata_timeout_secs),
                source_deadline: Duration::from_millis(config.dht.source_deadline_ms),
                source_k: config.dht.source_k,
                source_alpha: config.dht.source_alpha,
                source_query_timeout: Duration::from_secs(config.dht.source_query_timeout_secs),
                source_max_queries: config.dht.source_max_queries,
                race_peers: config.fetch.race_peers,
                failed_peer_sample_rate: config.fetch.failed_peer_sample_rate,
                transport_race_concurrent: config.fetch.transport_race_concurrent,
                connect_deadline: Duration::from_millis(config.fetch.connect_deadline_ms),
            },
        },
    );

    let report = report_loop(
        metrics.clone(),
        sightings.clone(),
        batch_writer.clone(),
        Duration::from_secs(config.report_interval_secs),
    );
    {
        // Phase 1 instrumentation: sample verify channel depth (no behavior change).
        let verify_gauge_tx = verify_tx.clone();
        let depth_metrics = metrics.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tick.tick().await;
                let depth = verify_gauge_tx
                    .max_capacity()
                    .saturating_sub(verify_gauge_tx.capacity());
                depth_metrics
                    .verify_channel_depth
                    .store(depth as u64, Ordering::Relaxed);
                let _ = depth_metrics
                    .verify_channel_depth_max
                    .fetch_max(depth as u64, Ordering::Relaxed);
            }
        });
        // Phase 2: fresh channel gauge (high-priority discovery queue).
        let fresh_gauge_tx = fresh_verify_tx.clone();
        let fresh_depth_metrics = metrics.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tick.tick().await;
                let depth = fresh_gauge_tx
                    .max_capacity()
                    .saturating_sub(fresh_gauge_tx.capacity());
                fresh_depth_metrics
                    .fresh_channel_depth
                    .store(depth as u64, Ordering::Relaxed);
                let _ = fresh_depth_metrics
                    .fresh_channel_depth_max
                    .fetch_max(depth as u64, Ordering::Relaxed);
            }
        });
    }
    let cache_cleanup = cache_cleanup_loop(
        peer_cache_cleanup,
        metrics.clone(),
        Duration::from_secs(config.cache.peer_cache_cleanup_interval_secs),
    );

    let shutdown_signal = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received, draining batch writer");
        let _ = shutdown_tx.send(());
    };

    tokio::select! {
        _ = sightings_run => {}
        _ = sightings_flush => {}
        _ = retry_run => {}
        _ = batch_run => {}
        _ = metrics_run => {}
        _ = peer_outcomes_run => {}
        _ = pipeline => {}
        _ = report => {}
        _ = cache_cleanup => {}
        _ = shutdown_signal => {}
    }

    tracing::info!("shutdown: draining pending writes");
    batch_writer.flush().await;
    sightings.flush().await;
    tracing::info!("shutdown complete");

    // Let logging guard flush remaining events
    tokio::time::sleep(Duration::from_millis(config.shutdown_flush_ms)).await;
    drop(_logging_guard);
}

fn log_effective_config(config: &Config) {
    tracing::info!(
        bind_addr = %config.bind_addr,
        nodes = config.nodes,
        port_base = config.port_base,
        worker_threads = config.worker_threads,
        channel_capacity = config.channel_capacity,
        walker_alpha = config.dht.walker_alpha,
        walker_interval_ms = config.dht.walker_interval_ms,
        find_node_response_percent = config.dht.find_node_response_percent,
        source_query_timeout = config.dht.source_query_timeout_secs,
        source_k = config.dht.source_k,
        source_alpha = config.dht.source_alpha,
        source_deadline_ms = config.dht.source_deadline_ms,
        source_max_queries = config.dht.source_max_queries,
        rate_limit = config.dht.rate_limit_per_sec,
        rate_limit_burst = config.dht.rate_limit_burst,
        global_fetch_limit = config.fetch.global_fetch_limit,
        pipeline_limit = config.fetch.pipeline_limit,
        race_peers = config.fetch.race_peers,
        max_conns_per_ip = config.fetch.max_connections_per_ip,
        fresh_channel_capacity = config.fetch.fresh_channel_capacity,
        metadata_timeout_secs = config.fetch.metadata_timeout_secs,
        tcp_timeout_secs = config.fetch.tcp_timeout_secs,
        utp_timeout_secs = config.fetch.utp_timeout_secs,
        utp_enabled = config.fetch.utp_enabled,
        max_retries = config.retry.max_retries,
        no_peers_terminal_on_first = config.retry.no_peers_terminal_on_first,
        no_metadata_max_retries = config.retry.no_metadata_max_retries,
        transport_race_concurrent = config.fetch.transport_race_concurrent,
        connect_deadline_ms = config.fetch.connect_deadline_ms,
        scheduler_claim_limit = config.retry.scheduler_claim_limit,
        scheduler_interval_secs = config.retry.scheduler_interval_secs,
        pg_pool_max = config.storage.pg_pool_max_connections,
        pg_pool_acquire_timeout = config.storage.pg_pool_acquire_timeout_secs,
        batch_flush_interval = config.storage.batch_flush_interval_secs,
        batch_flush_chunk = config.storage.batch_flush_chunk,
        torrent_batch_chunk = config.storage.torrent_batch_chunk,
        janitor_interval_secs = config.storage.janitor_interval_secs,
        janitor_batch_size = config.storage.janitor_batch_size,
        bloom_capacity = config.harvest.bloom_capacity,
        harvest_channel_capacity = config.harvest.harvest_channel_capacity,
        log_json = config.logging.log_json,
        log_dir = %config.logging.log_dir.display(),
        profile = %config.profile,
        "effective config"
    );
}

async fn utp_socket() -> Option<Arc<librqbit_utp::UtpSocketUdp>> {
    match librqbit_utp::UtpSocketUdp::new_udp(SocketAddr::from(([0, 0, 0, 0], 0))).await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "uTP socket unavailable; using TCP-only fetches");
            None
        }
    }
}

async fn spawn_node(
    config: &Config,
    node_index: usize,
    bootstrap: &[SocketAddr],
    metrics: Arc<Metrics>,
    harvest_tx: mpsc::Sender<HarvestEvent>,
    node_routers: &mut Vec<Arc<Router>>,
) {
    let data_dir = config.data_dir.join(format!("node_{node_index}"));
    std::fs::create_dir_all(&data_dir).expect("create node data dir");

    let identity = storage::identity::IdentityStore::load_or_create(
        &data_dir.join("identity.json"),
        config.external_ip,
        config.dht.sybil_count,
        config.dht.sybil_bep42_ratio,
    );
    let self_id = identity.self_id;
    let sybils = identity.sybils;

    let token_secret = load_or_create_secret(&data_dir.join("token_secret.bin"));
    let token = Arc::new(std::sync::RwLock::new(TokenGenerator::new(
        token_secret,
        Duration::from_secs(config.dht.token_window_secs),
    )));

    let bind = SocketAddr::new(config.bind_addr.ip(), config.port_base + node_index as u16);
    let sockets = net::bind_reuseport(bind, config.worker_threads)
        .expect("bind udp sockets")
        .into_iter()
        .map(Arc::new)
        .collect::<Vec<_>>();
    let self_addr = sockets[0].local_addr().expect("local addr");
    let send_socks: Arc<Vec<Arc<UdpSocket>>> = Arc::new(sockets.clone());

    let table = Arc::new(Mutex::new(RoutingTable::new(self_id)));
    load_routing_snapshot(&table, &data_dir.join("routing_table.bin"));

    let tx_table = Arc::new(TxTable::new());
    let router = Router::new(
        self_id,
        self_addr,
        sybils,
        config.external_ip,
        send_socks.clone(),
        tx_table.clone(),
        token.clone(),
        table.clone(),
        harvest_tx.clone(),
        metrics.clone(),
        config.dht.find_node_response_percent,
    );

    for sock in sockets {
        tokio::spawn(net::worker(sock, router.clone()));
    }

    let limiter = Arc::new(RateLimiter::new(
        config.dht.rate_limit_per_sec,
        config.dht.rate_limit_burst,
        config.dht.rate_limit_bucket_ttl_secs,
    ));
    let walker = Walker::new(
        router.clone(),
        limiter.clone(),
        bootstrap.to_vec(),
        config.dht.walker_alpha,
        Duration::from_millis(config.dht.walker_interval_ms),
        Duration::from_secs(config.dht.walker_query_timeout_secs),
        config.dht.walker_self_explore_prob,
        config.parse_nodes6,
    );
    walker.bootstrap(bootstrap).await;

    tokio::spawn(async move {
        walker.run().await;
    });
    tokio::spawn(limiter_sweep_loop(
        limiter.clone(),
        Duration::from_secs(config.rate_limit_sweep_interval_secs),
    ));
    let snap_path = data_dir.join("routing_table.bin");
    tokio::spawn(routing_snapshot_loop(
        table.clone(),
        snap_path,
        Duration::from_secs(config.dht.routing_snapshot_interval_secs),
    ));
    tokio::spawn(tx_cleanup(
        router.clone(),
        Duration::from_secs(config.dht.tx_cleanup_interval_secs),
        Duration::from_secs(config.dht.tx_entry_ttl_secs),
    ));
    tokio::spawn(token_rotation(
        token,
        Duration::from_secs(config.dht.token_window_secs.max(60)),
    ));

    node_routers.push(router);
}

fn load_or_create_secret(path: &std::path::Path) -> [u8; 32] {
    if let Ok(data) = std::fs::read(path)
        && let Ok(secret) = <[u8; 32]>::try_from(data.as_slice())
    {
        return secret;
    }
    let secret = rand::random::<[u8; 32]>();
    let _ = std::fs::write(path, secret);
    secret
}

fn load_routing_snapshot(table: &Arc<Mutex<RoutingTable>>, path: &std::path::Path) {
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let Ok(nodes) = bincode::deserialize::<Vec<NodeInfo>>(&data) else {
        return;
    };
    {
        let mut rt = table.lock().expect("routing table poisoned");
        for n in nodes {
            rt.insert(n);
        }
    }
    tracing::info!(
        restored = "routing_table",
        nodes = "loaded",
        "routing snapshot restored"
    );
}

async fn routing_snapshot_loop(table: Arc<Mutex<RoutingTable>>, path: PathBuf, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let nodes = table.lock().expect("routing table poisoned").all();
        if let Ok(data) = bincode::serialize(&nodes) {
            let tmp = path.with_extension("bin.tmp");
            if std::fs::write(&tmp, data).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

async fn flush_sightings(writer: Arc<SightingWriter>, mut rx: mpsc::Receiver<(NodeId, Source)>) {
    loop {
        match rx.recv().await {
            Some((ih, source)) => writer.push(ih, source),
            None => {
                tracing::warn!("discovery channel closed, flushing remaining sightings");
                writer.flush().await;
                std::future::pending::<()>().await;
            }
        }
    }
}

async fn limiter_sweep_loop(limiter: Arc<RateLimiter>, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let expired = limiter.sweep_expired();
        if expired > 0 {
            tracing::debug!(expired, "rate limiter: swept expired buckets");
        }
    }
}

async fn resolve_bootstrap(hosts: &[String]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for host in hosts {
        if let Ok(iter) = tokio::net::lookup_host(host).await {
            out.extend(iter);
        }
    }
    out
}

async fn report_loop(
    metrics: Arc<Metrics>,
    sightings: Arc<SightingWriter>,
    batch_writer: Arc<BatchWriter>,
    interval: Duration,
) {
    let mut prev = metrics.snapshot();
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let mut cur = metrics.snapshot();
        // Reset interval maximum via swap to current active (not zero); report
        // the maximum of swapped value and current active to cover race where
        // a task increments max between snapshot and swap.
        let current_active = metrics
            .pipeline_active
            .load(std::sync::atomic::Ordering::Relaxed);
        let swapped_max = metrics
            .pipeline_active_max_interval
            .swap(current_active, std::sync::atomic::Ordering::Relaxed);
        let interval_max = swapped_max.max(current_active);
        cur.pipeline_active_max_interval = interval_max;
        let dt = interval.as_secs_f64();
        let vph = (cur.verify_success - prev.verify_success) as f64 / dt * 3600.0;
        let uph = (cur.unique_infohashes - prev.unique_infohashes) as f64 / dt * 3600.0;
        let gp_bep42 = cur.inbound_get_peers_bep42;
        let gp_random = cur.inbound_get_peers_random;
        let gp_total = gp_bep42 + gp_random;
        let random_share = if gp_total > 0 {
            gp_random as f64 / gp_total as f64 * 100.0
        } else {
            0.0
        };
        tracing::info!(
            verified_per_hour = vph,
            unique_per_hour = uph,
            discovered_written = sightings.written(),
            metadata_written = batch_writer.torrents_written(),
            inbound_ping = cur.inbound_ping,
            inbound_find_node = cur.inbound_find_node,
            inbound_find_node_dropped = cur.inbound_find_node_dropped,
            inbound_get_peers = cur.inbound_get_peers,
            inbound_announce_peer = cur.inbound_announce_peer,
            inbound_invalid = cur.inbound_invalid,
            find_node_bep42 = cur.inbound_find_node_bep42,
            find_node_random = cur.inbound_find_node_random,
            get_peers_bep42 = gp_bep42,
            get_peers_random = gp_random,
            announce_bep42 = cur.inbound_announce_bep42,
            announce_random = cur.inbound_announce_random,
            announce_invalid_token = cur.inbound_announce_invalid_token,
            random_share_pct = random_share,
            outbound_queries = cur.outbound_queries,
            outbound_timeouts = cur.outbound_timeouts,
            send_dropped = cur.send_dropped,
            tokens_issued = cur.tokens_issued,
            harvested = cur.infohashes_harvested,
            unique_total = cur.unique_infohashes,
            routing_table = cur.routing_table_len,
            tx_table = cur.tx_table_len,
            verify_attempts = cur.verify_attempts,
            verify_success = cur.verify_success,
            verify_fail = cur.verify_fail,
            verify_timeouts = cur.verify_timeouts,
            fetch_attempts = cur.fetch_attempts,
            source_queries = cur.source_queries,
            source_responses = cur.source_responses,
            source_peers = cur.source_peers_returned,
            source_timeout = cur.source_timeout,
            source_all_timeout = cur.source_all_timeout,
            source_no_peers = cur.source_no_peers,
            tcp_attempts = cur.tcp_attempts,
            utp_attempts = cur.utp_attempts,
            tcp_connect_ok = cur.tcp_connect_ok,
            utp_connect_ok = cur.utp_connect_ok,
            tcp_metadata_ok = cur.tcp_metadata_ok,
            utp_metadata_ok = cur.utp_metadata_ok,
            tcp_connect_actual = cur.tcp_connect_actual,
            utp_connect_actual = cur.utp_connect_actual,
            source_returned_peers = cur.source_returned_peers,
            source_filtered_by_cache = cur.source_filtered_by_cache,
            source_no_values = cur.source_no_values,
            source_deadline_hits = cur.source_deadline_hits,
            source_deadline_peers = cur.source_deadline_peers,
            fetch_connect_timeout = cur.fetch_connect_timeout,
            fetch_connect_io = cur.fetch_connect_io,
            fetch_handshake = cur.fetch_handshake,
            fetch_no_extension = cur.fetch_no_extension,
            fetch_reject = cur.fetch_reject,
            fetch_bad_piece = cur.fetch_bad_piece,
            fetch_io = cur.fetch_io,
            sha1_mismatch = cur.sha1_mismatch,
            announce_attempts = cur.announce_attempts,
            announce_success = cur.announce_success,
            inbound_announce_valid = cur.inbound_announce_valid,
            inbound_announce_invalid_token = cur.inbound_announce_invalid_token,
            walker_steps = cur.walker_steps,
            walker_queries = cur.walker_queries,
            walker_ok = cur.walker_ok,
            walker_nodes_returned = cur.walker_nodes_returned,
            walker_self_target = cur.walker_self_target,
            walker_random_target = cur.walker_random_target,
            walker_sybil_target = cur.walker_sybil_target,
            routing_insert_calls = cur.routing_insert_calls,
            routing_nodes_added = cur.routing_nodes_added,
            routing_buckets_used = cur.routing_buckets_used,
            routing_new_ids = cur.routing_new_ids,
            routing_rejected = cur.routing_rejected,
            log_dropped = cur.log_dropped,
            harvest_try_send_dropped = cur.harvest_try_send_dropped,
            harvest_sighting_tx_dropped = cur.harvest_sighting_tx_dropped,
            scheduler_send_blocked = cur.scheduler_send_blocked,
            scheduler_claims = cur.scheduler_claims,
            scheduler_claimed_fresh = cur.scheduler_claimed_fresh,
            scheduler_claimed_retry = cur.scheduler_claimed_retry,
            verify_channel_depth = cur.verify_channel_depth,
            verify_channel_depth_max = cur.verify_channel_depth_max,
            fresh_channel_dropped = cur.fresh_channel_dropped,
            fresh_channel_depth = cur.fresh_channel_depth,
            fresh_channel_depth_max = cur.fresh_channel_depth_max,
            scheduler_skipped_backpressure = cur.scheduler_skipped_backpressure,
            fresh_dequeued_total = cur.fresh_dequeued_total,
            retry_dequeued_total = cur.retry_dequeued_total,
            announce_dequeued_total = cur.announce_dequeued_total,
            pipeline_dequeued_total = cur.pipeline_dequeued_total,
            pipeline_spawned_total = cur.pipeline_spawned_total,
            pipeline_completed_total = cur.pipeline_completed_total,
            pipeline_cancelled_total = cur.pipeline_cancelled_total,
            pipeline_active = cur.pipeline_active,
            pipeline_active_max_interval = cur.pipeline_active_max_interval,
            pipeline_permit_wait_micros_total = cur.pipeline_permit_wait_micros_total,
            pipeline_permit_acquisitions_total = cur.pipeline_permit_acquisitions_total,
            pipeline_task_micros_total = cur.pipeline_task_micros_total,
            pipeline_no_peers_total = cur.pipeline_no_peers_total,
            verify_source_micros_total = cur.verify_source_micros_total,
            verify_source_completed_total = cur.verify_source_completed_total,
            fetch_permit_wait_micros_total = cur.fetch_permit_wait_micros_total,
            fetch_permit_acquisitions_total = cur.fetch_permit_acquisitions_total,
            per_ip_wait_micros_total = cur.per_ip_wait_micros_total,
            per_ip_acquisitions_total = cur.per_ip_acquisitions_total,
            transport_connect_micros_total = cur.transport_connect_micros_total,
            transport_connect_completed_total = cur.transport_connect_completed_total,
            metadata_exchange_micros_total = cur.metadata_exchange_micros_total,
            metadata_exchange_completed_total = cur.metadata_exchange_completed_total,
            result_handling_micros_total = cur.result_handling_micros_total,
            result_handling_completed_total = cur.result_handling_completed_total,
            source_active = cur.source_active,
            fetch_active = cur.fetch_active,
            metadata_active = cur.metadata_active,
        );
        tracing::info!(
            source_direct_accepted = cur.source_direct_accepted_total,
            source_direct_attempts = cur.source_direct_attempts_total,
            source_direct_connect_ok = cur.source_direct_connect_ok_total,
            source_direct_connect_timeout = cur.source_direct_connect_timeout_total,
            source_direct_connect_io = cur.source_direct_connect_io_total,
            source_direct_metadata_ok = cur.source_direct_metadata_ok_total,
            source_direct_metadata_fail = cur.source_direct_metadata_fail_total,
            source_direct_verified = cur.source_direct_verified_total,
            source_announce_cache_accepted = cur.source_announce_cache_accepted_total,
            source_announce_cache_attempts = cur.source_announce_cache_attempts_total,
            source_announce_cache_connect_ok = cur.source_announce_cache_connect_ok_total,
            source_announce_cache_connect_timeout = cur.source_announce_cache_connect_timeout_total,
            source_announce_cache_connect_io = cur.source_announce_cache_connect_io_total,
            source_announce_cache_metadata_ok = cur.source_announce_cache_metadata_ok_total,
            source_announce_cache_metadata_fail = cur.source_announce_cache_metadata_fail_total,
            source_announce_cache_verified = cur.source_announce_cache_verified_total,
            source_dht_accepted = cur.source_dht_accepted_total,
            source_dht_attempts = cur.source_dht_attempts_total,
            source_dht_connect_ok = cur.source_dht_connect_ok_total,
            source_dht_connect_timeout = cur.source_dht_connect_timeout_total,
            source_dht_connect_io = cur.source_dht_connect_io_total,
            source_dht_metadata_ok = cur.source_dht_metadata_ok_total,
            source_dht_metadata_fail = cur.source_dht_metadata_fail_total,
            source_dht_verified = cur.source_dht_verified_total,
            "candidate source metrics"
        );
        tracing::info!(
            lead_tasks = cur.lead_tasks_total,
            lead_tasks_dht_started = cur.lead_tasks_dht_started_total,
            lead_tasks_queries = cur.lead_tasks_queries_total,
            lead_tasks_lead_verified = cur.lead_tasks_lead_verified_total,
            lead_tasks_dht_verified = cur.lead_tasks_dht_verified_total,
            non_lead_tasks = cur.non_lead_tasks_total,
            non_lead_tasks_queries = cur.non_lead_tasks_queries_total,
            lead_success_le_250ms = cur.lead_success_le_250ms_total,
            lead_success_le_500ms = cur.lead_success_le_500ms_total,
            lead_success_le_1000ms = cur.lead_success_le_1000ms_total,
            lead_success_le_2000ms = cur.lead_success_le_2000ms_total,
            lead_success_gt_2000ms = cur.lead_success_gt_2000ms_total,
            lead_failure_le_250ms = cur.lead_failure_le_250ms_total,
            lead_failure_le_500ms = cur.lead_failure_le_500ms_total,
            lead_failure_le_1000ms = cur.lead_failure_le_1000ms_total,
            lead_failure_le_2000ms = cur.lead_failure_le_2000ms_total,
            lead_failure_gt_2000ms = cur.lead_failure_gt_2000ms_total,
            "lead metrics"
        );
        prev = cur;
    }
}

async fn tx_cleanup(router: Arc<Router>, tick_interval: Duration, entry_ttl: Duration) {
    let mut tick = tokio::time::interval(tick_interval);
    loop {
        tick.tick().await;
        router.cleanup_tx(entry_ttl);
    }
}

async fn token_rotation(token: Arc<std::sync::RwLock<TokenGenerator>>, window: Duration) {
    let mut tick = tokio::time::interval(window);
    loop {
        tick.tick().await;
        token
            .write()
            .expect("token generator poisoned")
            .rotate(rand::random::<[u8; 32]>());
    }
}

async fn cache_cleanup_loop(cache: Arc<PeerCache>, metrics: Arc<Metrics>, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        let evicted = cache.evict_expired();
        metrics
            .peer_cache_size
            .store(cache.len() as u64, std::sync::atomic::Ordering::Relaxed);
        if evicted > 0 {
            metrics
                .peer_cache_evictions
                .fetch_add(evicted as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
