use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

// ── Defaults ────────────────────────────────────────────────────────────────
// These MUST match current production values (captured from zerone on
// 2026-08-25). Changing a default here changes runtime behavior. If you need
// to tune, use config/*.toml or CRAW_* env vars — never edit these silently.

const DEFAULT_CONFIG_PATH: &str = "config";

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub external_ip: Option<IpAddr>,
    pub bootstrap: Vec<String>,
    pub worker_threads: usize,
    pub nodes: usize,
    pub port_base: u16,
    pub data_dir: PathBuf,
    pub trace_sample_rate: f64,
    pub debug_ih: Option<String>,
    pub parse_nodes6: bool,
    pub channel_capacity: usize,
    pub report_interval_secs: u64,
    pub rate_limit_sweep_interval_secs: u64,
    pub shutdown_flush_ms: u64,
    pub profile: String,

    pub dht: DhtConfig,
    pub fetch: FetchConfig,
    pub retry: RetryConfig,
    pub storage: StorageConfig,
    pub cache: CacheConfig,
    pub harvest: HarvestConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DhtConfig {
    pub walker_alpha: usize,
    pub walker_interval_ms: u64,
    pub walker_query_timeout_secs: u64,
    pub walker_self_explore_prob: f64,
    pub sybil_count: usize,
    pub sybil_bep42_ratio: f64,
    pub find_node_response_percent: u8,
    pub routing_k: usize,
    pub source_k: usize,
    pub source_alpha: usize,
    pub source_query_timeout_secs: u64,
    pub source_deadline_ms: u64,
    pub source_max_queries: usize,
    pub routing_snapshot_interval_secs: u64,
    pub tx_cleanup_interval_secs: u64,
    pub tx_entry_ttl_secs: u64,
    pub tx_collision_retries: u32,
    pub rate_limit_per_sec: f64,
    pub rate_limit_burst: f64,
    pub rate_limit_bucket_ttl_secs: u64,
    pub token_window_secs: u64,
    pub linux_mmsg_receive: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FetchConfig {
    pub tcp_timeout_secs: u64,
    pub utp_timeout_secs: u64,
    pub metadata_timeout_secs: u64,
    pub race_peers: usize,
    pub global_fetch_limit: usize,
    pub max_connections_per_ip: usize,
    pub utp_enabled: bool,
    pub max_message_len: usize,
    pub max_pieces: usize,
    pub failed_peer_sample_rate: u64,
    pub fresh_channel_capacity: usize,
    pub transport_race_concurrent: bool,
    pub connect_deadline_ms: u64,
    pub pipeline_limit: usize,
    pub lead_source_grace_ms: u64,
    pub conn_limiter_ttl_secs: u64,
    pub conn_limiter_max_entries: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetryConfig {
    pub max_retries: i32,
    pub backoff_sequence_secs: Vec<u64>,
    pub scheduler_interval_secs: u64,
    pub scheduler_claim_limit: i64,
    pub scheduler_fresh_ratio: f64,
    pub stale_verifying_timeout_secs: u64,
    pub no_peers_terminal_on_first: bool,
    pub no_peers_max_retries: i32,
    pub no_metadata_max_retries: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub pg_pool_max_connections: u32,
    pub pg_pool_min_connections: u32,
    pub pg_pool_acquire_timeout_secs: u64,
    pub batch_flush_interval_secs: u64,
    pub batch_flush_chunk: usize,
    pub torrent_batch_chunk: usize,
    pub batch_initial_capacity: usize,
    pub sighting_flush_interval_ms: u64,
    pub sighting_chunk_size: usize,
    pub metrics_flush_interval_secs: u64,
    pub peer_outcomes_flush_interval_secs: u64,
    pub peer_outcomes_chunk_size: usize,
    pub janitor_interval_secs: u64,
    pub janitor_dead_retention_secs: u64,
    pub janitor_verified_retention_secs: u64,
    pub janitor_peer_outcomes_retention_secs: u64,
    pub janitor_batch_size: i64,
    pub janitor_batch_sleep_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub peer_cache_ttl_secs: u64,
    pub peer_cache_max_entries: usize,
    pub peer_cache_cleanup_interval_secs: u64,
    pub peer_cache_failure_threshold: u8,
    pub announce_cache_ttl_secs: u64,
    pub announce_cache_max_entries: usize,
    pub announce_cache_initial_capacity: usize,
    pub announce_cache_shards: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HarvestConfig {
    pub bloom_capacity: usize,
    pub bloom_fp_rate: f64,
    pub announce_bloom_ratio: f64,
    pub announce_bloom_min: usize,
    pub harvest_channel_capacity: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub log_dir: PathBuf,
    pub log_json: bool,
    pub log_file_max_bytes: u64,
    pub log_total_max_bytes: u64,
    pub log_flush_interval_ms: u64,
    pub log_buffer_capacity: usize,
    pub log_batch_size: usize,
    pub log_max_file_age_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = PathBuf::from("data");
        Config {
            bind_addr: "0.0.0.0:6882".parse().unwrap(),
            external_ip: None,
            bootstrap: vec![
                "router.bittorrent.com:6881".to_string(),
                "router.utorrent.com:6881".to_string(),
                "dht.transmissionbt.com:6881".to_string(),
                "dht.libtorrent.org:25401".to_string(),
                "router.bitcomet.com:6881".to_string(),
            ],
            worker_threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            nodes: 1,
            port_base: 6882,
            data_dir: data_dir.clone(),
            trace_sample_rate: 0.0,
            debug_ih: None,
            parse_nodes6: false,
            channel_capacity: 65536,
            report_interval_secs: 15,
            rate_limit_sweep_interval_secs: 60,
            shutdown_flush_ms: 500,
            profile: "production".to_string(),
            dht: DhtConfig::default(),
            fetch: FetchConfig::default(),
            retry: RetryConfig::default(),
            storage: StorageConfig::default(),
            cache: CacheConfig::default(),
            harvest: HarvestConfig::default(),
            logging: LoggingConfig::default_with_data_dir(&data_dir),
        }
    }
}

impl Default for DhtConfig {
    fn default() -> Self {
        DhtConfig {
            walker_alpha: 3,
            walker_interval_ms: 250,
            walker_query_timeout_secs: 5,
            walker_self_explore_prob: 0.25,
            sybil_count: 128,
            sybil_bep42_ratio: 0.125,
            find_node_response_percent: 100,
            routing_k: 8,
            source_k: 8,
            source_alpha: 3,
            source_query_timeout_secs: 5,
            source_deadline_ms: 25000,
            source_max_queries: 24,
            routing_snapshot_interval_secs: 60,
            tx_cleanup_interval_secs: 10,
            tx_entry_ttl_secs: 30,
            tx_collision_retries: 32,
            rate_limit_per_sec: 8.0,
            rate_limit_burst: 64.0,
            rate_limit_bucket_ttl_secs: 600,
            token_window_secs: 300,
            linux_mmsg_receive: true,
        }
    }
}

impl Default for FetchConfig {
    fn default() -> Self {
        FetchConfig {
            tcp_timeout_secs: 3,
            utp_timeout_secs: 5,
            metadata_timeout_secs: 25,
            race_peers: 8,
            global_fetch_limit: 1200,
            max_connections_per_ip: 4,
            utp_enabled: true,
            max_message_len: 16 * 1024 * 1024,
            max_pieces: 4096,
            failed_peer_sample_rate: 500,
            fresh_channel_capacity: 65536,
            transport_race_concurrent: true,
            connect_deadline_ms: 10000,
            pipeline_limit: 4000,
            lead_source_grace_ms: 1000,
            conn_limiter_ttl_secs: 60,
            conn_limiter_max_entries: 1_000_000,
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig {
            max_retries: 4,
            backoff_sequence_secs: vec![60, 300, 1800, 7200, 43200],
            scheduler_interval_secs: 15,
            scheduler_claim_limit: 1000,
            scheduler_fresh_ratio: 0.7,
            stale_verifying_timeout_secs: 300,
            no_peers_terminal_on_first: true,
            no_peers_max_retries: 2,
            no_metadata_max_retries: 1,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            pg_pool_max_connections: 128,
            pg_pool_min_connections: 1,
            pg_pool_acquire_timeout_secs: 30,
            batch_flush_interval_secs: 1,
            batch_flush_chunk: 5000,
            torrent_batch_chunk: 2000,
            batch_initial_capacity: 4096,
            sighting_flush_interval_ms: 500,
            sighting_chunk_size: 256,
            metrics_flush_interval_secs: 60,
            peer_outcomes_flush_interval_secs: 30,
            peer_outcomes_chunk_size: 256,
            janitor_interval_secs: 1800,
            janitor_dead_retention_secs: 86400,
            janitor_verified_retention_secs: 3600,
            janitor_peer_outcomes_retention_secs: 172800,
            janitor_batch_size: 25000,
            janitor_batch_sleep_ms: 100,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            peer_cache_ttl_secs: 300,
            peer_cache_max_entries: 500_000,
            peer_cache_cleanup_interval_secs: 60,
            peer_cache_failure_threshold: 2,
            announce_cache_ttl_secs: 600,
            announce_cache_max_entries: 250_000,
            announce_cache_initial_capacity: 1024,
            announce_cache_shards: 64,
        }
    }
}

impl Default for HarvestConfig {
    fn default() -> Self {
        HarvestConfig {
            bloom_capacity: 1_000_000,
            bloom_fp_rate: 0.001,
            announce_bloom_ratio: 0.25,
            announce_bloom_min: 64,
            harvest_channel_capacity: 10_000,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig::default_with_dir(PathBuf::from("data/logs"))
    }
}

impl LoggingConfig {
    fn default_with_dir(log_dir: PathBuf) -> Self {
        LoggingConfig {
            log_dir,
            log_json: false,
            log_file_max_bytes: 50_000_000,
            log_total_max_bytes: 500_000_000,
            log_flush_interval_ms: 500,
            log_buffer_capacity: 8192,
            log_batch_size: 1000,
            log_max_file_age_secs: 3600,
        }
    }

    fn default_with_data_dir(data_dir: &PathBuf) -> Self {
        LoggingConfig::default_with_dir(data_dir.join("logs"))
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

fn env_string(key: &str, default: Option<String>) -> Option<String> {
    std::env::var(key).ok().or(default)
}

impl Config {
    /// Load config with precedence:
    ///   built-in defaults -> default.toml -> {profile}.toml -> CRAW_* env
    pub fn load() -> Config {
        let mut cfg = Config::default();

        // 1. Load default.toml (if present) on top of built-in defaults.
        let profile = std::env::var("CRAW_PROFILE").unwrap_or_else(|_| "production".into());
        cfg.profile = profile.clone();
        let config_dir = std::env::var("CRAW_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONFIG_PATH));

        let default_path = config_dir.join("default.toml");
        if let Ok(contents) = std::fs::read_to_string(&default_path)
            && let Ok(toml_cfg) = toml::from_str::<PartialConfig>(&contents)
        {
            toml_cfg.merge_into(&mut cfg);
        }

        // 2. Load profile TOML (if present).
        let profile_path = config_dir.join(format!("{profile}.toml"));
        if let Ok(contents) = std::fs::read_to_string(&profile_path)
            && let Ok(toml_cfg) = toml::from_str::<PartialConfig>(&contents)
        {
            toml_cfg.merge_into(&mut cfg);
        }

        // 3. Apply env overrides (highest precedence).
        cfg.apply_env();

        cfg
    }

    fn apply_env(&mut self) {
        if let Some(addr) = std::env::var("CRAW_BIND").ok().and_then(|v| v.parse().ok()) {
            self.bind_addr = addr;
        }
        if let Some(addr) = std::env::var("CRAW_EXTERNAL_IP")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.external_ip = Some(addr);
        }
        if let Ok(boot) = std::env::var("CRAW_BOOTSTRAP") {
            self.bootstrap = boot.split(',').map(|s| s.trim().to_string()).collect();
        }
        self.worker_threads = env_usize("CRAW_WORKERS", self.worker_threads).max(1);
        self.nodes = env_usize("CRAW_NODES", self.nodes).max(1);
        self.port_base = env_u64("CRAW_PORT_BASE", 0) as u16;
        if self.port_base == 0 {
            self.port_base = self.bind_addr.port();
        }
        self.channel_capacity = env_usize("CRAW_CHANNEL_CAPACITY", self.channel_capacity);
        self.report_interval_secs = env_u64("CRAW_REPORT_INTERVAL", self.report_interval_secs);
        self.rate_limit_sweep_interval_secs = env_u64(
            "CRAW_RATE_LIMIT_SWEEP_INTERVAL",
            self.rate_limit_sweep_interval_secs,
        );
        if let Ok(dir) = std::env::var("CRAW_DATA_DIR") {
            self.data_dir = PathBuf::from(dir);
            // If data_dir changes and logging log_dir wasn't explicitly set
            // (still defaulting relative to the old data dir), re-derive.
            if self.logging.log_dir.to_string_lossy() == "data/logs" {
                self.logging.log_dir = self.data_dir.join("logs");
            }
        }
        self.trace_sample_rate = env_f64("CRAW_TRACE_SAMPLE_RATE", self.trace_sample_rate);
        self.debug_ih = env_string("CRAW_DEBUG_IH", self.debug_ih.clone());
        self.parse_nodes6 = env_bool("CRAW_PARSE_NODES6", self.parse_nodes6);

        // dht
        self.dht.walker_alpha = env_usize("CRAW_WALKER_ALPHA", self.dht.walker_alpha);
        self.dht.walker_interval_ms =
            env_u64("CRAW_WALKER_INTERVAL_MS", self.dht.walker_interval_ms);
        self.dht.walker_self_explore_prob = env_f64(
            "CRAW_WALKER_SELF_EXPLORE_PROB",
            self.dht.walker_self_explore_prob,
        );
        self.dht.sybil_count = env_usize("CRAW_SYBILS", self.dht.sybil_count);
        self.dht.sybil_bep42_ratio = env_f64("CRAW_SYBIL_BEP42_RATIO", self.dht.sybil_bep42_ratio);
        self.dht.source_deadline_ms =
            env_u64("CRAW_SOURCE_DEADLINE_MS", self.dht.source_deadline_ms);
        self.dht.source_query_timeout_secs = env_u64(
            "CRAW_SOURCE_QUERY_TIMEOUT",
            self.dht.source_query_timeout_secs,
        );
        self.dht.rate_limit_per_sec = env_f64("CRAW_RATE_LIMIT", self.dht.rate_limit_per_sec);
        self.dht.token_window_secs = env_u64("CRAW_TOKEN_WINDOW", self.dht.token_window_secs);
        self.dht.find_node_response_percent = env_u64(
            "CRAW_FIND_NODE_RESPONSE_PERCENT",
            self.dht.find_node_response_percent as u64,
        ) as u8;
        self.dht.linux_mmsg_receive =
            env_bool("CRAW_LINUX_MMSG_RECEIVE", self.dht.linux_mmsg_receive);

        // fetch
        self.fetch.global_fetch_limit =
            env_usize("CRAW_FETCH_LIMIT", self.fetch.global_fetch_limit);
        self.fetch.race_peers = env_usize("CRAW_RACE_PEERS", self.fetch.race_peers);
        self.fetch.metadata_timeout_secs = env_u64(
            "CRAW_FETCH_TIMEOUT_MS",
            self.fetch.metadata_timeout_secs * 1000,
        ) / 1000;
        self.fetch.utp_enabled = env_bool("CRAW_UTP_ENABLED", self.fetch.utp_enabled);
        self.fetch.transport_race_concurrent = env_bool(
            "CRAW_TRANSPORT_RACE_CONCURRENT",
            self.fetch.transport_race_concurrent,
        );
        self.fetch.connect_deadline_ms =
            env_u64("CRAW_CONNECT_DEADLINE_MS", self.fetch.connect_deadline_ms);
        self.fetch.pipeline_limit = env_usize("CRAW_PIPELINE_LIMIT", self.fetch.pipeline_limit);
        self.fetch.lead_source_grace_ms =
            env_u64("CRAW_LEAD_SOURCE_GRACE_MS", self.fetch.lead_source_grace_ms);

        // storage / janitor
        self.storage.janitor_interval_secs =
            env_u64("CRAW_JANITOR_INTERVAL", self.storage.janitor_interval_secs);
        self.storage.janitor_batch_size = env_u64(
            "CRAW_JANITOR_BATCH_SIZE",
            self.storage.janitor_batch_size as u64,
        ) as i64;
        self.storage.janitor_batch_sleep_ms = env_u64(
            "CRAW_JANITOR_BATCH_SLEEP_MS",
            self.storage.janitor_batch_sleep_ms,
        );
        self.storage.janitor_dead_retention_secs = env_u64(
            "CRAW_JANITOR_DEAD_RETENTION_SECS",
            self.storage.janitor_dead_retention_secs,
        );
        self.storage.janitor_peer_outcomes_retention_secs = env_u64(
            "CRAW_JANITOR_PEER_OUTCOMES_RETENTION_SECS",
            self.storage.janitor_peer_outcomes_retention_secs,
        );

        // fetch (tcp/utp timeout)
        self.fetch.tcp_timeout_secs = env_u64("CRAW_TCP_TIMEOUT_SECS", self.fetch.tcp_timeout_secs);
        self.fetch.utp_timeout_secs = env_u64("CRAW_UTP_TIMEOUT_SECS", self.fetch.utp_timeout_secs);
        self.fetch.conn_limiter_ttl_secs = env_u64(
            "CRAW_CONN_LIMITER_TTL_SECS",
            self.fetch.conn_limiter_ttl_secs,
        );
        self.fetch.conn_limiter_max_entries = env_usize(
            "CRAW_CONN_LIMITER_MAX_ENTRIES",
            self.fetch.conn_limiter_max_entries,
        );

        // cache
        self.cache.peer_cache_ttl_secs =
            env_u64("CRAW_PEER_CACHE_TTL_SECS", self.cache.peer_cache_ttl_secs);
        self.cache.peer_cache_max_entries = env_usize(
            "CRAW_PEER_CACHE_MAX_ENTRIES",
            self.cache.peer_cache_max_entries,
        );
        self.cache.peer_cache_failure_threshold = env_u64(
            "CRAW_PEER_CACHE_FAILURE_THRESHOLD",
            self.cache.peer_cache_failure_threshold as u64,
        ) as u8;
        self.cache.announce_cache_ttl_secs =
            env_u64("CRAW_ANNOUNCE_CACHE_TTL_SECS", self.cache.announce_cache_ttl_secs);
        self.cache.announce_cache_max_entries = env_usize(
            "CRAW_ANNOUNCE_CACHE_MAX_ENTRIES",
            self.cache.announce_cache_max_entries,
        );

        // harvest
        self.harvest.harvest_channel_capacity = env_usize(
            "CRAW_HARVEST_CHANNEL_CAPACITY",
            self.harvest.harvest_channel_capacity,
        );

        // retry
        self.retry.no_peers_terminal_on_first = env_bool(
            "CRAW_NO_PEERS_TERMINAL_ON_FIRST",
            self.retry.no_peers_terminal_on_first,
        );
        self.retry.no_peers_max_retries = env_usize(
            "CRAW_NO_PEERS_MAX_RETRIES",
            self.retry.no_peers_max_retries as usize,
        ) as i32;

        // logging
        if let Ok(dir) = std::env::var("CRAW_LOG_DIR") {
            self.logging.log_dir = PathBuf::from(dir);
        }
        self.logging.log_json = env_bool("CRAW_LOG_JSON", self.logging.log_json);
        self.logging.log_file_max_bytes =
            env_u64("CRAW_LOG_FILE_MAX", self.logging.log_file_max_bytes);
        self.logging.log_total_max_bytes =
            env_u64("CRAW_LOG_TOTAL_MAX", self.logging.log_total_max_bytes);
        self.logging.log_flush_interval_ms =
            env_u64("CRAW_LOG_FLUSH_MS", self.logging.log_flush_interval_ms);
        self.logging.log_buffer_capacity =
            env_usize("CRAW_LOG_BUFFER", self.logging.log_buffer_capacity);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.fetch.lead_source_grace_ms > 60_000 {
            return Err(format!(
                "fetch.lead_source_grace_ms ({}) must be <= 60000",
                self.fetch.lead_source_grace_ms
            ));
        }
        if self.dht.source_query_timeout_secs * 1000 > self.dht.source_deadline_ms {
            return Err(format!(
                "source_query_timeout_secs ({}) * 1000 > source_deadline_ms ({})",
                self.dht.source_query_timeout_secs, self.dht.source_deadline_ms
            ));
        }
        if self.fetch.global_fetch_limit == 0 {
            return Err("fetch.global_fetch_limit must be > 0".into());
        }
        if self.fetch.pipeline_limit == 0 {
            return Err("fetch.pipeline_limit must be > 0".into());
        }
        if self.fetch.pipeline_limit < self.fetch.global_fetch_limit {
            return Err(format!(
                "fetch.pipeline_limit ({}) must be >= fetch.global_fetch_limit ({})",
                self.fetch.pipeline_limit, self.fetch.global_fetch_limit
            ));
        }
        if self.fetch.race_peers == 0 {
            return Err("fetch.race_peers must be > 0".into());
        }
        if self.fetch.fresh_channel_capacity == 0 {
            return Err("fetch.fresh_channel_capacity must be > 0".into());
        }
        if self.retry.backoff_sequence_secs.is_empty() {
            return Err("retry.backoff_sequence_secs must not be empty".into());
        }
        if self.dht.walker_alpha == 0 {
            return Err("dht.walker_alpha must be > 0".into());
        }
        if self.dht.source_alpha == 0 {
            return Err("dht.source_alpha must be > 0".into());
        }
        if self.dht.source_max_queries == 0 {
            return Err("dht.source_max_queries must be > 0".into());
        }
        if self.storage.pg_pool_max_connections == 0 {
            return Err("storage.pg_pool_max_connections must be > 0".into());
        }
        if self.channel_capacity == 0 {
            return Err("channel_capacity must be > 0".into());
        }
        if self.harvest.bloom_capacity == 0 {
            return Err("harvest.bloom_capacity must be > 0".into());
        }
        if self.harvest.harvest_channel_capacity == 0 {
            return Err("harvest.harvest_channel_capacity must be > 0".into());
        }
        if self.logging.log_buffer_capacity == 0 {
            return Err("logging.log_buffer_capacity must be > 0".into());
        }
        if self.dht.find_node_response_percent > 100 {
            return Err("dht.find_node_response_percent must be <= 100".into());
        }
        if self.dht.find_node_response_percent == 0 {
            return Err("dht.find_node_response_percent must be > 0".into());
        }
        Ok(())
    }
}

// ── TOML partial override ───────────────────────────────────────────────────
// All fields optional; only fields present in the TOML file override. This is
// what makes profile files sparse overrides of the built-in defaults.

#[derive(Debug, Default, Deserialize)]
struct PartialConfig {
    #[serde(default)]
    bind_addr: Option<SocketAddr>,
    #[serde(default)]
    external_ip: Option<IpAddr>,
    #[serde(default)]
    bootstrap: Option<Vec<String>>,
    #[serde(default)]
    worker_threads: Option<usize>,
    #[serde(default)]
    nodes: Option<usize>,
    #[serde(default)]
    port_base: Option<u16>,
    #[serde(default)]
    data_dir: Option<PathBuf>,
    #[serde(default)]
    trace_sample_rate: Option<f64>,
    #[serde(default)]
    debug_ih: Option<String>,
    #[serde(default)]
    parse_nodes6: Option<bool>,
    #[serde(default)]
    channel_capacity: Option<usize>,
    #[serde(default)]
    report_interval_secs: Option<u64>,
    #[serde(default)]
    rate_limit_sweep_interval_secs: Option<u64>,
    #[serde(default)]
    shutdown_flush_ms: Option<u64>,
    #[serde(default)]
    dht: Option<PartialDht>,
    #[serde(default)]
    fetch: Option<PartialFetch>,
    #[serde(default)]
    retry: Option<PartialRetry>,
    #[serde(default)]
    storage: Option<PartialStorage>,
    #[serde(default)]
    cache: Option<PartialCache>,
    #[serde(default)]
    harvest: Option<PartialHarvest>,
    #[serde(default)]
    logging: Option<PartialLogging>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialDht {
    #[serde(default)]
    walker_alpha: Option<usize>,
    #[serde(default)]
    walker_interval_ms: Option<u64>,
    #[serde(default)]
    walker_query_timeout_secs: Option<u64>,
    #[serde(default)]
    walker_self_explore_prob: Option<f64>,
    #[serde(default)]
    sybil_count: Option<usize>,
    #[serde(default)]
    sybil_bep42_ratio: Option<f64>,
    find_node_response_percent: Option<u8>,
    #[serde(default)]
    routing_k: Option<usize>,
    #[serde(default)]
    source_k: Option<usize>,
    #[serde(default)]
    source_alpha: Option<usize>,
    #[serde(default)]
    source_query_timeout_secs: Option<u64>,
    #[serde(default)]
    source_deadline_ms: Option<u64>,
    #[serde(default)]
    source_max_queries: Option<usize>,
    #[serde(default)]
    routing_snapshot_interval_secs: Option<u64>,
    #[serde(default)]
    tx_cleanup_interval_secs: Option<u64>,
    #[serde(default)]
    tx_entry_ttl_secs: Option<u64>,
    #[serde(default)]
    tx_collision_retries: Option<u32>,
    #[serde(default)]
    rate_limit_per_sec: Option<f64>,
    #[serde(default)]
    rate_limit_burst: Option<f64>,
    #[serde(default)]
    rate_limit_bucket_ttl_secs: Option<u64>,
    #[serde(default)]
    token_window_secs: Option<u64>,
    #[serde(default)]
    linux_mmsg_receive: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialFetch {
    #[serde(default)]
    tcp_timeout_secs: Option<u64>,
    #[serde(default)]
    utp_timeout_secs: Option<u64>,
    #[serde(default)]
    metadata_timeout_secs: Option<u64>,
    #[serde(default)]
    race_peers: Option<usize>,
    #[serde(default)]
    global_fetch_limit: Option<usize>,
    #[serde(default)]
    max_connections_per_ip: Option<usize>,
    #[serde(default)]
    utp_enabled: Option<bool>,
    #[serde(default)]
    max_message_len: Option<usize>,
    #[serde(default)]
    max_pieces: Option<usize>,
    #[serde(default)]
    failed_peer_sample_rate: Option<u64>,
    #[serde(default)]
    fresh_channel_capacity: Option<usize>,
    #[serde(default)]
    transport_race_concurrent: Option<bool>,
    #[serde(default)]
    connect_deadline_ms: Option<u64>,
    #[serde(default)]
    pipeline_limit: Option<usize>,
    #[serde(default)]
    lead_source_grace_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialRetry {
    #[serde(default)]
    max_retries: Option<i32>,
    #[serde(default)]
    backoff_sequence_secs: Option<Vec<u64>>,
    #[serde(default)]
    scheduler_interval_secs: Option<u64>,
    #[serde(default)]
    scheduler_claim_limit: Option<i64>,
    #[serde(default)]
    scheduler_fresh_ratio: Option<f64>,
    #[serde(default)]
    stale_verifying_timeout_secs: Option<u64>,
    #[serde(default)]
    no_peers_terminal_on_first: Option<bool>,
    #[serde(default)]
    no_peers_max_retries: Option<i32>,
    #[serde(default)]
    no_metadata_max_retries: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialStorage {
    #[serde(default)]
    pg_pool_max_connections: Option<u32>,
    #[serde(default)]
    pg_pool_min_connections: Option<u32>,
    #[serde(default)]
    pg_pool_acquire_timeout_secs: Option<u64>,
    #[serde(default)]
    batch_flush_interval_secs: Option<u64>,
    #[serde(default)]
    batch_flush_chunk: Option<usize>,
    #[serde(default)]
    torrent_batch_chunk: Option<usize>,
    #[serde(default)]
    batch_initial_capacity: Option<usize>,
    #[serde(default)]
    sighting_flush_interval_ms: Option<u64>,
    #[serde(default)]
    sighting_chunk_size: Option<usize>,
    #[serde(default)]
    metrics_flush_interval_secs: Option<u64>,
    #[serde(default)]
    peer_outcomes_flush_interval_secs: Option<u64>,
    #[serde(default)]
    peer_outcomes_chunk_size: Option<usize>,
    #[serde(default)]
    janitor_interval_secs: Option<u64>,
    #[serde(default)]
    janitor_dead_retention_secs: Option<u64>,
    #[serde(default)]
    janitor_verified_retention_secs: Option<u64>,
    #[serde(default)]
    janitor_peer_outcomes_retention_secs: Option<u64>,
    #[serde(default)]
    janitor_batch_size: Option<i64>,
    #[serde(default)]
    janitor_batch_sleep_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialCache {
    #[serde(default)]
    peer_cache_ttl_secs: Option<u64>,
    #[serde(default)]
    peer_cache_max_entries: Option<usize>,
    #[serde(default)]
    peer_cache_cleanup_interval_secs: Option<u64>,
    #[serde(default)]
    peer_cache_failure_threshold: Option<u8>,
    #[serde(default)]
    announce_cache_ttl_secs: Option<u64>,
    #[serde(default)]
    announce_cache_max_entries: Option<usize>,
    #[serde(default)]
    announce_cache_initial_capacity: Option<usize>,
    #[serde(default)]
    announce_cache_shards: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialHarvest {
    #[serde(default)]
    bloom_capacity: Option<usize>,
    #[serde(default)]
    bloom_fp_rate: Option<f64>,
    #[serde(default)]
    announce_bloom_ratio: Option<f64>,
    #[serde(default)]
    announce_bloom_min: Option<usize>,
    #[serde(default)]
    harvest_channel_capacity: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct PartialLogging {
    #[serde(default)]
    log_dir: Option<PathBuf>,
    #[serde(default)]
    log_json: Option<bool>,
    #[serde(default)]
    log_file_max_bytes: Option<u64>,
    #[serde(default)]
    log_total_max_bytes: Option<u64>,
    #[serde(default)]
    log_flush_interval_ms: Option<u64>,
    #[serde(default)]
    log_buffer_capacity: Option<usize>,
    #[serde(default)]
    log_batch_size: Option<usize>,
    #[serde(default)]
    log_max_file_age_secs: Option<u64>,
}

impl PartialConfig {
    fn merge_into(self, cfg: &mut Config) {
        if let Some(v) = self.bind_addr {
            cfg.bind_addr = v;
        }
        if let Some(v) = self.external_ip {
            cfg.external_ip = Some(v);
        }
        if let Some(v) = self.bootstrap {
            cfg.bootstrap = v;
        }
        if let Some(v) = self.worker_threads {
            cfg.worker_threads = v;
        }
        if let Some(v) = self.nodes {
            cfg.nodes = v;
        }
        if let Some(v) = self.port_base {
            cfg.port_base = v;
        }
        if let Some(v) = self.data_dir {
            cfg.data_dir = v;
        }
        if let Some(v) = self.trace_sample_rate {
            cfg.trace_sample_rate = v;
        }
        if let Some(v) = self.debug_ih {
            cfg.debug_ih = Some(v);
        }
        if let Some(v) = self.parse_nodes6 {
            cfg.parse_nodes6 = v;
        }
        if let Some(v) = self.channel_capacity {
            cfg.channel_capacity = v;
        }
        if let Some(v) = self.report_interval_secs {
            cfg.report_interval_secs = v;
        }
        if let Some(v) = self.rate_limit_sweep_interval_secs {
            cfg.rate_limit_sweep_interval_secs = v;
        }
        if let Some(v) = self.shutdown_flush_ms {
            cfg.shutdown_flush_ms = v;
        }
        if let Some(v) = self.dht {
            v.merge_into(&mut cfg.dht);
        }
        if let Some(v) = self.fetch {
            v.merge_into(&mut cfg.fetch);
        }
        if let Some(v) = self.retry {
            v.merge_into(&mut cfg.retry);
        }
        if let Some(v) = self.storage {
            v.merge_into(&mut cfg.storage);
        }
        if let Some(v) = self.cache {
            v.merge_into(&mut cfg.cache);
        }
        if let Some(v) = self.harvest {
            v.merge_into(&mut cfg.harvest);
        }
        if let Some(v) = self.logging {
            v.merge_into(&mut cfg.logging);
        }
    }
}

impl PartialDht {
    fn merge_into(self, cfg: &mut DhtConfig) {
        if let Some(v) = self.walker_alpha {
            cfg.walker_alpha = v;
        }
        if let Some(v) = self.walker_interval_ms {
            cfg.walker_interval_ms = v;
        }
        if let Some(v) = self.walker_query_timeout_secs {
            cfg.walker_query_timeout_secs = v;
        }
        if let Some(v) = self.walker_self_explore_prob {
            cfg.walker_self_explore_prob = v;
        }
        if let Some(v) = self.sybil_count {
            cfg.sybil_count = v;
        }
        if let Some(v) = self.find_node_response_percent {
            cfg.find_node_response_percent = v;
        }
        if let Some(v) = self.sybil_bep42_ratio {
            cfg.sybil_bep42_ratio = v;
        }
        if let Some(v) = self.routing_k {
            cfg.routing_k = v;
        }
        if let Some(v) = self.source_k {
            cfg.source_k = v;
        }
        if let Some(v) = self.source_alpha {
            cfg.source_alpha = v;
        }
        if let Some(v) = self.source_query_timeout_secs {
            cfg.source_query_timeout_secs = v;
        }
        if let Some(v) = self.source_deadline_ms {
            cfg.source_deadline_ms = v;
        }
        if let Some(v) = self.source_max_queries {
            cfg.source_max_queries = v;
        }
        if let Some(v) = self.routing_snapshot_interval_secs {
            cfg.routing_snapshot_interval_secs = v;
        }
        if let Some(v) = self.tx_cleanup_interval_secs {
            cfg.tx_cleanup_interval_secs = v;
        }
        if let Some(v) = self.tx_entry_ttl_secs {
            cfg.tx_entry_ttl_secs = v;
        }
        if let Some(v) = self.tx_collision_retries {
            cfg.tx_collision_retries = v;
        }
        if let Some(v) = self.rate_limit_per_sec {
            cfg.rate_limit_per_sec = v;
        }
        if let Some(v) = self.rate_limit_burst {
            cfg.rate_limit_burst = v;
        }
        if let Some(v) = self.rate_limit_bucket_ttl_secs {
            cfg.rate_limit_bucket_ttl_secs = v;
        }
        if let Some(v) = self.token_window_secs {
            cfg.token_window_secs = v;
        }
        if let Some(v) = self.linux_mmsg_receive {
            cfg.linux_mmsg_receive = v;
        }
    }
}

impl PartialFetch {
    fn merge_into(self, cfg: &mut FetchConfig) {
        if let Some(v) = self.tcp_timeout_secs {
            cfg.tcp_timeout_secs = v;
        }
        if let Some(v) = self.utp_timeout_secs {
            cfg.utp_timeout_secs = v;
        }
        if let Some(v) = self.metadata_timeout_secs {
            cfg.metadata_timeout_secs = v;
        }
        if let Some(v) = self.race_peers {
            cfg.race_peers = v;
        }
        if let Some(v) = self.global_fetch_limit {
            cfg.global_fetch_limit = v;
        }
        if let Some(v) = self.max_connections_per_ip {
            cfg.max_connections_per_ip = v;
        }
        if let Some(v) = self.utp_enabled {
            cfg.utp_enabled = v;
        }
        if let Some(v) = self.max_message_len {
            cfg.max_message_len = v;
        }
        if let Some(v) = self.max_pieces {
            cfg.max_pieces = v;
        }
        if let Some(v) = self.failed_peer_sample_rate {
            cfg.failed_peer_sample_rate = v;
        }
        if let Some(v) = self.fresh_channel_capacity {
            cfg.fresh_channel_capacity = v;
        }
        if let Some(v) = self.transport_race_concurrent {
            cfg.transport_race_concurrent = v;
        }
        if let Some(v) = self.connect_deadline_ms {
            cfg.connect_deadline_ms = v;
        }
        if let Some(v) = self.pipeline_limit {
            cfg.pipeline_limit = v;
        }
        if let Some(v) = self.lead_source_grace_ms {
            cfg.lead_source_grace_ms = v;
        }
    }
}

impl PartialRetry {
    fn merge_into(self, cfg: &mut RetryConfig) {
        if let Some(v) = self.max_retries {
            cfg.max_retries = v;
        }
        if let Some(v) = self.backoff_sequence_secs {
            cfg.backoff_sequence_secs = v;
        }
        if let Some(v) = self.scheduler_interval_secs {
            cfg.scheduler_interval_secs = v;
        }
        if let Some(v) = self.scheduler_claim_limit {
            cfg.scheduler_claim_limit = v;
        }
        if let Some(v) = self.scheduler_fresh_ratio {
            cfg.scheduler_fresh_ratio = v;
        }
        if let Some(v) = self.stale_verifying_timeout_secs {
            cfg.stale_verifying_timeout_secs = v;
        }
        if let Some(v) = self.no_peers_terminal_on_first {
            cfg.no_peers_terminal_on_first = v;
        }
        if let Some(v) = self.no_peers_max_retries {
            cfg.no_peers_max_retries = v;
        }
        if let Some(v) = self.no_metadata_max_retries {
            cfg.no_metadata_max_retries = v;
        }
    }
}

impl PartialStorage {
    fn merge_into(self, cfg: &mut StorageConfig) {
        if let Some(v) = self.pg_pool_max_connections {
            cfg.pg_pool_max_connections = v;
        }
        if let Some(v) = self.pg_pool_min_connections {
            cfg.pg_pool_min_connections = v;
        }
        if let Some(v) = self.pg_pool_acquire_timeout_secs {
            cfg.pg_pool_acquire_timeout_secs = v;
        }
        if let Some(v) = self.batch_flush_interval_secs {
            cfg.batch_flush_interval_secs = v;
        }
        if let Some(v) = self.batch_flush_chunk {
            cfg.batch_flush_chunk = v;
        }
        if let Some(v) = self.torrent_batch_chunk {
            cfg.torrent_batch_chunk = v;
        }
        if let Some(v) = self.batch_initial_capacity {
            cfg.batch_initial_capacity = v;
        }
        if let Some(v) = self.sighting_flush_interval_ms {
            cfg.sighting_flush_interval_ms = v;
        }
        if let Some(v) = self.sighting_chunk_size {
            cfg.sighting_chunk_size = v;
        }
        if let Some(v) = self.metrics_flush_interval_secs {
            cfg.metrics_flush_interval_secs = v;
        }
        if let Some(v) = self.peer_outcomes_flush_interval_secs {
            cfg.peer_outcomes_flush_interval_secs = v;
        }
        if let Some(v) = self.peer_outcomes_chunk_size {
            cfg.peer_outcomes_chunk_size = v;
        }
        if let Some(v) = self.janitor_interval_secs {
            cfg.janitor_interval_secs = v;
        }
        if let Some(v) = self.janitor_dead_retention_secs {
            cfg.janitor_dead_retention_secs = v;
        }
        if let Some(v) = self.janitor_verified_retention_secs {
            cfg.janitor_verified_retention_secs = v;
        }
        if let Some(v) = self.janitor_peer_outcomes_retention_secs {
            cfg.janitor_peer_outcomes_retention_secs = v;
        }
        if let Some(v) = self.janitor_batch_size {
            cfg.janitor_batch_size = v;
        }
        if let Some(v) = self.janitor_batch_sleep_ms {
            cfg.janitor_batch_sleep_ms = v;
        }
    }
}

impl PartialCache {
    fn merge_into(self, cfg: &mut CacheConfig) {
        if let Some(v) = self.peer_cache_ttl_secs {
            cfg.peer_cache_ttl_secs = v;
        }
        if let Some(v) = self.peer_cache_max_entries {
            cfg.peer_cache_max_entries = v;
        }
        if let Some(v) = self.peer_cache_cleanup_interval_secs {
            cfg.peer_cache_cleanup_interval_secs = v;
        }
        if let Some(v) = self.peer_cache_failure_threshold {
            cfg.peer_cache_failure_threshold = v;
        }
        if let Some(v) = self.announce_cache_ttl_secs {
            cfg.announce_cache_ttl_secs = v;
        }
        if let Some(v) = self.announce_cache_max_entries {
            cfg.announce_cache_max_entries = v;
        }
        if let Some(v) = self.announce_cache_initial_capacity {
            cfg.announce_cache_initial_capacity = v;
        }
        if let Some(v) = self.announce_cache_shards {
            cfg.announce_cache_shards = v;
        }
    }
}

impl PartialHarvest {
    fn merge_into(self, cfg: &mut HarvestConfig) {
        if let Some(v) = self.bloom_capacity {
            cfg.bloom_capacity = v;
        }
        if let Some(v) = self.bloom_fp_rate {
            cfg.bloom_fp_rate = v;
        }
        if let Some(v) = self.announce_bloom_ratio {
            cfg.announce_bloom_ratio = v;
        }
        if let Some(v) = self.announce_bloom_min {
            cfg.announce_bloom_min = v;
        }
        if let Some(v) = self.harvest_channel_capacity {
            cfg.harvest_channel_capacity = v;
        }
    }
}

impl PartialLogging {
    fn merge_into(self, cfg: &mut LoggingConfig) {
        if let Some(v) = self.log_dir {
            cfg.log_dir = v;
        }
        if let Some(v) = self.log_json {
            cfg.log_json = v;
        }
        if let Some(v) = self.log_file_max_bytes {
            cfg.log_file_max_bytes = v;
        }
        if let Some(v) = self.log_total_max_bytes {
            cfg.log_total_max_bytes = v;
        }
        if let Some(v) = self.log_flush_interval_ms {
            cfg.log_flush_interval_ms = v;
        }
        if let Some(v) = self.log_buffer_capacity {
            cfg.log_buffer_capacity = v;
        }
        if let Some(v) = self.log_batch_size {
            cfg.log_batch_size = v;
        }
        if let Some(v) = self.log_max_file_age_secs {
            cfg.log_max_file_age_secs = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_production() {
        let c = Config::default();
        // Verified against zerone production env 2026-08-25
        assert_eq!(c.fetch.global_fetch_limit, 1200);
        assert_eq!(c.fetch.metadata_timeout_secs, 25);
        assert_eq!(c.dht.walker_alpha, 3);
        assert_eq!(c.dht.walker_interval_ms, 250);
        assert_eq!(c.dht.source_deadline_ms, 25000);
        assert_eq!(c.dht.source_max_queries, 24);
        assert_eq!(c.dht.source_query_timeout_secs, 5);
        assert_eq!(c.fetch.race_peers, 8);
        assert_eq!(c.dht.rate_limit_per_sec, 8.0);
        assert_eq!(c.dht.rate_limit_burst, 64.0);
        assert_eq!(c.channel_capacity, 65536);
        assert_eq!(c.fetch.fresh_channel_capacity, 65536);
        assert_eq!(c.report_interval_secs, 15);
        assert_eq!(c.rate_limit_sweep_interval_secs, 60);
        assert_eq!(c.shutdown_flush_ms, 500);
        assert_eq!(c.retry.scheduler_claim_limit, 1000);
        assert_eq!(c.storage.pg_pool_max_connections, 128);
        assert_eq!(c.logging.log_dir, PathBuf::from("data/logs"));
        assert_eq!(c.fetch.max_connections_per_ip, 4);
        assert_eq!(c.retry.no_peers_terminal_on_first, true);
        assert_eq!(c.storage.torrent_batch_chunk, 2000);
        assert_eq!(c.storage.janitor_interval_secs, 1800);
        assert_eq!(c.storage.janitor_batch_size, 25000);
        assert_eq!(c.fetch.transport_race_concurrent, true);
        assert_eq!(c.fetch.connect_deadline_ms, 10000);
        assert_eq!(c.fetch.pipeline_limit, 4000);
        assert_eq!(c.fetch.lead_source_grace_ms, 1000);
        assert_eq!(c.harvest.harvest_channel_capacity, 10_000);
        assert_eq!(c.dht.find_node_response_percent, 100);
        assert!(c.dht.linux_mmsg_receive);
    }

    #[test]
    fn source_timeout_le_deadline() {
        let c = Config::default();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn lead_source_grace_ms_validation_and_defaults() {
        let c = Config::default();
        assert_eq!(c.fetch.lead_source_grace_ms, 1000);
        assert!(c.validate().is_ok());

        let mut c_zero = Config::default();
        c_zero.fetch.lead_source_grace_ms = 0;
        assert!(c_zero.validate().is_ok());

        let mut c_valid_max = Config::default();
        c_valid_max.fetch.lead_source_grace_ms = 60_000;
        assert!(c_valid_max.validate().is_ok());

        let mut c_invalid = Config::default();
        c_invalid.fetch.lead_source_grace_ms = 60_001;
        assert!(c_invalid.validate().is_err());
    }
}

#[test]
fn toml_files_parse() {
    // Verify the shipped TOML files decode against the partial schema.
    for name in ["default", "production", "development"] {
        let path = PathBuf::from("config").join(format!("{name}.toml"));
        let contents =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}.toml missing: {e}"));
        let p = toml::from_str::<PartialConfig>(&contents)
            .unwrap_or_else(|e| panic!("{name}.toml invalid: {e}"));
        assert!(p.bind_addr.is_none() || p.bind_addr.is_some());
    }
}
