use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

pub struct Config {
    pub bind_addr: SocketAddr,
    pub external_ip: Option<IpAddr>,
    pub bootstrap: Vec<String>,
    pub worker_threads: usize,
    pub sybil_count: usize,
    pub token_window_secs: u64,
    pub bloom_capacity: usize,
    pub walker_alpha: usize,
    pub walker_interval_ms: u64,
    pub global_fetch_limit: usize,
    pub race_peers: usize,
    pub data_dir: PathBuf,
    pub rate_limit_per_sec: f64,
    pub trace_sample_rate: f64,
    pub debug_ih: Option<String>,
    pub parse_nodes6: bool,
    pub nodes: usize,
    pub port_base: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind_addr: "0.0.0.0:6881".parse().unwrap(),
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
            sybil_count: 16,
            token_window_secs: 300,
            bloom_capacity: 1_000_000,
            walker_alpha: 16,
            walker_interval_ms: 20,
            global_fetch_limit: 128,
            race_peers: 8,
            data_dir: PathBuf::from("data"),
            rate_limit_per_sec: 8.0,
            trace_sample_rate: 0.0,
            debug_ih: None,
            parse_nodes6: false,
            nodes: 1,
            port_base: 6881,
        }
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

impl Config {
    pub fn from_env() -> Config {
        let mut c = Config::default();
        if let Some(addr) = std::env::var("CRAW_BIND").ok().and_then(|v| v.parse().ok()) {
            c.bind_addr = addr;
        }
        if let Some(addr) = std::env::var("CRAW_EXTERNAL_IP")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            c.external_ip = Some(addr);
        }
        if let Ok(boot) = std::env::var("CRAW_BOOTSTRAP") {
            c.bootstrap = boot.split(',').map(|s| s.trim().to_string()).collect();
        }
        c.worker_threads = env_usize("CRAW_WORKERS", c.worker_threads).max(1);
        c.sybil_count = env_usize("CRAW_SYBILS", c.sybil_count);
        c.token_window_secs = env_u64("CRAW_TOKEN_WINDOW", c.token_window_secs);
        c.bloom_capacity = env_usize("CRAW_BLOOM_CAPACITY", c.bloom_capacity);
        c.walker_alpha = env_usize("CRAW_WALKER_ALPHA", c.walker_alpha);
        c.walker_interval_ms = env_u64("CRAW_WALKER_INTERVAL_MS", c.walker_interval_ms);
        c.global_fetch_limit = env_usize("CRAW_FETCH_LIMIT", c.global_fetch_limit);
        c.race_peers = env_usize("CRAW_RACE_PEERS", c.race_peers);
        if let Ok(dir) = std::env::var("CRAW_DATA_DIR") {
            c.data_dir = PathBuf::from(dir);
        }
        if let Some(v) = std::env::var("CRAW_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            c.rate_limit_per_sec = v;
        }
        if let Some(v) = std::env::var("CRAW_TRACE_SAMPLE_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            c.trace_sample_rate = v;
        }
        c.debug_ih = std::env::var("CRAW_DEBUG_IH").ok();
        c.parse_nodes6 = std::env::var("CRAW_PARSE_NODES6")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        c.nodes = env_usize("CRAW_NODES", c.nodes).max(1);
        c.port_base = env_u64("CRAW_PORT_BASE", 0).max(0) as u16;
        if c.port_base == 0 {
            c.port_base = c.bind_addr.port();
        }
        c
    }
}
