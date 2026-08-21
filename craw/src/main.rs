mod config;
mod dht;
mod harvest;
mod krpc;
mod metrics;
mod net;
mod router;
mod storage;
mod verify;

use crate::config::Config;
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
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

    if std::env::args().any(|a| a == "--backfill") {
        storage::backfill::run(&pool, &config.data_dir)
            .await
            .expect("backfill");
        tracing::info!("backfill complete");
        return;
    }

    let metrics = Arc::new(Metrics::new());

    let identity = storage::identity::IdentityStore::load_or_create(
        &config.data_dir.join("identity.json"),
        config.external_ip,
        config.sybil_count,
    );
    let self_id = identity.self_id;
    let sybils = identity.sybils;

    let token_secret = load_or_create_secret(&config.data_dir.join("token_secret.bin"));
    let token = Arc::new(Mutex::new(TokenGenerator::new(
        token_secret,
        Duration::from_secs(config.token_window_secs),
    )));

    let mut sockets = net::bind_reuseport(config.bind_addr, config.worker_threads)
        .expect("bind udp sockets")
        .into_iter()
        .map(Arc::new)
        .collect::<Vec<_>>();
    let send_sock = sockets.swap_remove(0);
    let self_addr = send_sock.local_addr().expect("local addr");

    tracing::info!(
        self_addr = %self_addr,
        workers = config.worker_threads,
        sybils = sybils.len(),
        "crawler starting"
    );

    let (discovery_tx, discovery_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (verify_tx, verify_rx) = mpsc::channel(CHANNEL_CAPACITY);

    let table = Arc::new(Mutex::new(RoutingTable::new(self_id)));
    load_routing_snapshot(&table, &config.data_dir.join("routing_table.bin"));

    let tx_table = Arc::new(TxTable::new());
    let harvester = Arc::new(Mutex::new(Harvester::new(
        config.bloom_capacity,
        discovery_tx,
        verify_tx.clone(),
        metrics.clone(),
    )));

    let router = Router::new(
        self_id,
        self_addr,
        sybils,
        send_sock.clone(),
        tx_table,
        token.clone(),
        table.clone(),
        harvester,
        metrics.clone(),
    );

    for sock in sockets {
        tokio::spawn(net::worker(sock, router.clone()));
    }
    tokio::spawn(net::worker(send_sock, router.clone()));

    let limiter = Arc::new(RateLimiter::new(config.rate_limit_per_sec, 64.0));
    let walker = Walker::new(
        router.clone(),
        limiter,
        config.walker_alpha,
        Duration::from_millis(config.walker_interval_ms),
    );

    let bootstrap = resolve_bootstrap(&config.bootstrap).await;
    walker.bootstrap(bootstrap).await;

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
        .run_scheduler(verify_tx.clone(), Duration::from_secs(30));

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

    let metrics_writer = Arc::new(storage::metrics_writer::MetricsWriter::new(
        pool,
        metrics.clone(),
    ));
    let metrics_run = metrics_writer.clone().run(Duration::from_secs(60));

    let pipeline = verify::run_pipeline(
        verify_rx,
        router.clone(),
        utp_socket().await,
        metrics.clone(),
        batch_writer.clone(),
        VerifyConfig {
            global_limit: config.global_fetch_limit,
            race_peers: config.race_peers,
        },
    );

    let routing_snapshot = routing_snapshot_loop(
        table.clone(),
        config.data_dir.join("routing_table.bin"),
        ROUTING_SNAPSHOT_INTERVAL,
    );

    let report = report_loop(
        metrics.clone(),
        sightings.clone(),
        batch_writer.clone(),
        Duration::from_secs(15),
    );
    let cleanup = tx_cleanup(router.clone());
    let token_rotate = token_rotation(token, Duration::from_secs(config.token_window_secs.max(60)));

    let shutdown_signal = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received, draining batch writer");
        let _ = shutdown_tx.send(());
    };

    tokio::select! {
        _ = walker.run() => {}
        _ = sightings_run => {}
        _ = sightings_flush => {}
        _ = retry_run => {}
        _ = batch_run => {}
        _ = metrics_run => {}
        _ = pipeline => {}
        _ = routing_snapshot => {}
        _ = report => {}
        _ = cleanup => {}
        _ = token_rotate => {}
        _ = shutdown_signal => {}
    }
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
    while let Some((ih, source)) = rx.recv().await {
        writer.push(ih, source);
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

async fn token_rotation(token: Arc<Mutex<TokenGenerator>>, window: Duration) {
    let mut tick = tokio::time::interval(window);
    loop {
        tick.tick().await;
        token
            .lock()
            .expect("token generator poisoned")
            .rotate(rand::random::<[u8; 32]>());
    }
}
