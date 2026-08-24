mod config;
mod dht;
mod harvest;
mod krpc;
mod metrics;
mod net;
mod router;
mod storage;
mod trace;
mod verify;

use crate::config::Config;
use crate::trace::TraceConfig;
use crate::dht::routing_table::{NodeInfo, RoutingTable};
use crate::dht::walker::Walker;
use crate::harvest::{Harvester, Source};
use crate::krpc::NodeId;
use crate::krpc::token::TokenGenerator;
use crate::krpc::tx_state::TxTable;
use crate::metrics::Metrics;
use crate::net::rate_limit::RateLimiter;
use crate::router::Router;
use crate::storage::batch_writer::BatchWriter;
use crate::storage::jobs::VerifyStore;
use crate::storage::sightings::SightingWriter;
use crate::verify::VerifyConfig;
use crate::verify::peer_cache::PeerCache;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc};

const CHANNEL_CAPACITY: usize = 65536;
const ROUTING_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();

    tracing::info!(
        git_hash = env!("CRAW_GIT_HASH"),
        target_arch = env!("CRAW_TARGET_ARCH"),
        target_os = env!("CRAW_TARGET_OS"),
        "crawler starting"
    );

    std::fs::create_dir_all(&config.data_dir).expect("create data dir");

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable is required");
    let pool = storage::pg::connect(&database_url)
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

    let metrics = Arc::new(Metrics::new());
    crate::trace::TRACE_CONFIG
        .set(TraceConfig::new(config.trace_sample_rate, config.debug_ih.clone()))
        .unwrap_or(());

    let (discovery_tx, discovery_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (verify_tx, verify_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (announce_tx, announce_rx) = mpsc::channel(CHANNEL_CAPACITY);

    let harvester = Arc::new(Mutex::new(Harvester::new(
        config.bloom_capacity,
        discovery_tx,
        verify_tx.clone(),
        announce_tx,
        metrics.clone(),
    )));

    let peer_cache = Arc::new(PeerCache::new(Duration::from_secs(600)));
    let peer_cache_cleanup = peer_cache.clone();

    let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
        broadcast::channel(1);

    tracing::info!(
        nodes = config.nodes,
        port_base = config.port_base,
        workers_per_node = config.worker_threads,
        sybils_per_node = config.sybil_count,
        fetch_limit = config.global_fetch_limit,
        race_peers = config.race_peers,
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
            harvester.clone(),
            &mut node_routers,
        ).await;
        tracing::info!(node = i, "dht node started");
    }
    let node_routers: Arc<Vec<Arc<Router>>> = Arc::new(node_routers);

    let sightings = Arc::new(SightingWriter::new(pool.clone()));
    let sightings_run = sightings.clone().run(Duration::from_millis(500));
    let sightings_flush = flush_sightings(sightings.clone(), discovery_rx);

    let verify_store = Arc::new(VerifyStore::new(
        pool.clone(),
        vec![
            Duration::from_secs(60),
            Duration::from_secs(300),
            Duration::from_secs(1800),
            Duration::from_secs(7200),
            Duration::from_secs(43200),
        ],
    ));
    let retry_run = verify_store
        .clone()
        .run_scheduler(verify_tx.clone(), Duration::from_secs(15));

    let batch_writer = Arc::new(BatchWriter::new(
        pool.clone(),
        vec![
            Duration::from_secs(60),
            Duration::from_secs(300),
            Duration::from_secs(1800),
            Duration::from_secs(7200),
            Duration::from_secs(43200),
        ],
    ));

    let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
    let batch_run = batch_writer
        .clone()
        .run(Duration::from_secs(1), shutdown_tx.subscribe());

    let janitor_pool = pool.clone();
    tokio::spawn(async move {
        storage::janitor::run(&janitor_pool).await;
        let mut tick = tokio::time::interval(Duration::from_secs(4 * 3600));
        loop {
            tick.tick().await;
            storage::janitor::run(&janitor_pool).await;
        }
    });

    let metrics_writer = Arc::new(storage::metrics_writer::MetricsWriter::new(
        pool.clone(),
        metrics.clone(),
    ));
    let metrics_run = metrics_writer.clone().run(Duration::from_secs(60));

    let announce_peer_cache = Arc::new(verify::AnnouncePeerCache::default());
    let peer_outcomes = Arc::new(crate::storage::peer_outcomes::PeerOutcomeWriter::new(pool.clone()));
    let peer_outcomes_run = peer_outcomes.clone().run(Duration::from_secs(30));

    let pipeline = verify::run_pipeline(
        verify_rx,
        announce_rx,
        node_routers,
        if config.utp_enabled { utp_socket().await } else { None },
        metrics.clone(),
        batch_writer.clone(),
        peer_cache.clone(),
        announce_peer_cache,
        peer_outcomes,
        VerifyConfig {
            global_limit: config.global_fetch_limit,
            race_peers: config.race_peers,
            fetch_timeout_ms: config.fetch_timeout_ms,
        },
    );

    let report = report_loop(
        metrics.clone(),
        sightings.clone(),
        batch_writer.clone(),
        Duration::from_secs(15),
    );
    let cache_cleanup = cache_cleanup_loop(peer_cache_cleanup, metrics.clone(), Duration::from_secs(60));

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
    harvester: Arc<Mutex<Harvester>>,
    node_routers: &mut Vec<Arc<Router>>,
) {
    let data_dir = config.data_dir.join(format!("node_{node_index}"));
    std::fs::create_dir_all(&data_dir).expect("create node data dir");

    let identity = storage::identity::IdentityStore::load_or_create(
        &data_dir.join("identity.json"),
        config.external_ip,
        config.sybil_count,
    );
    let self_id = identity.self_id;
    let sybils = identity.sybils;

    let token_secret = load_or_create_secret(&data_dir.join("token_secret.bin"));
    let token = Arc::new(std::sync::RwLock::new(TokenGenerator::new(
        token_secret,
        Duration::from_secs(config.token_window_secs),
    )));

    let bind = SocketAddr::new(
        config.bind_addr.ip(),
        config.port_base + node_index as u16,
    );
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
        harvester.clone(),
        metrics.clone(),
    );

    for sock in sockets {
        tokio::spawn(net::worker(sock, router.clone()));
    }

    let limiter = Arc::new(RateLimiter::new(config.rate_limit_per_sec, 64.0));
    let walker = Walker::new(
        router.clone(),
        limiter.clone(),
        bootstrap.to_vec(),
        config.walker_alpha,
        Duration::from_millis(config.walker_interval_ms),
        config.parse_nodes6,
    );
    walker.bootstrap(bootstrap).await;

    tokio::spawn(async move {
        walker.run().await;
    });
    tokio::spawn(limiter_sweep_loop(limiter.clone()));
    let snap_path = data_dir.join("routing_table.bin");
    tokio::spawn(routing_snapshot_loop(table.clone(), snap_path, ROUTING_SNAPSHOT_INTERVAL));
    tokio::spawn(tx_cleanup(router.clone()));
    tokio::spawn(token_rotation(token, Duration::from_secs(config.token_window_secs.max(60))));

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

async fn limiter_sweep_loop(limiter: Arc<RateLimiter>) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
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
        let cur = metrics.snapshot();
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
        );
        prev = cur;
    }
}

async fn tx_cleanup(router: Arc<Router>) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    loop {
        tick.tick().await;
        router.cleanup_tx(Duration::from_secs(30));
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
        metrics.peer_cache_size.store(cache.len() as u64, std::sync::atomic::Ordering::Relaxed);
        if evicted > 0 {
            metrics.peer_cache_evictions.fetch_add(evicted as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
