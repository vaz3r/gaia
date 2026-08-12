//! Shared configuration primitives used across irontide crates.
//!
//! These types were lifted down from `irontide-session` (M242) so that the
//! extracted `irontide-settings` crate and `irontide-session` can both depend
//! on them without forming a dependency cycle. Runtime logic that *uses* these
//! types (proxy sockets, the choker, the rate limiter, the alert stream) stays
//! in `irontide-session` and re-exports each type at its original module path.

use serde::{Deserialize, Serialize};

// ── Proxy ─────────────────────────────────────────────────────────────

/// Supported proxy protocols (matching libtorrent).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProxyType {
    /// No proxy (direct connections).
    #[default]
    None,
    /// SOCKS4 proxy (no authentication, no UDP).
    Socks4,
    /// SOCKS5 proxy without authentication.
    Socks5,
    /// SOCKS5 proxy with username/password authentication.
    Socks5Password,
    /// HTTP CONNECT proxy without authentication.
    Http,
    /// HTTP CONNECT proxy with username/password authentication.
    HttpPassword,
}

/// Proxy connection settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Proxy protocol to use.
    pub proxy_type: ProxyType,
    /// Proxy server hostname or IP address.
    pub hostname: String,
    /// Proxy server port.
    pub port: u16,
    /// Username for authenticated proxy types.
    pub username: Option<String>,
    /// Password for authenticated proxy types.
    pub password: Option<String>,
    /// Route peer connections (incl. web seeds) through proxy.
    #[serde(default = "default_true")]
    pub proxy_peer_connections: bool,
    /// Route tracker HTTP connections through proxy.
    #[serde(default = "default_true")]
    pub proxy_tracker_connections: bool,
    /// Resolve hostnames through proxy (SOCKS5/HTTP only).
    #[serde(default = "default_true")]
    pub proxy_hostnames: bool,
    /// Include local endpoint in SOCKS5 UDP ASSOCIATE.
    #[serde(default)]
    pub socks5_udp_send_local_ep: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            proxy_type: ProxyType::None,
            hostname: String::new(),
            port: 0,
            username: None,
            password: None,
            proxy_peer_connections: true,
            proxy_tracker_connections: true,
            proxy_hostnames: true,
            socks5_udp_send_local_ep: false,
        }
    }
}

impl ProxyConfig {
    /// Format as a URL suitable for `reqwest::Proxy::all()`.
    #[must_use]
    pub fn to_url(&self) -> String {
        let scheme = match self.proxy_type {
            ProxyType::None => return String::new(),
            ProxyType::Socks4 => "socks4",
            ProxyType::Socks5 | ProxyType::Socks5Password => "socks5",
            ProxyType::Http | ProxyType::HttpPassword => "http",
        };

        match (&self.username, &self.password) {
            (Some(u), Some(p))
                if self.proxy_type == ProxyType::Socks5Password
                    || self.proxy_type == ProxyType::HttpPassword =>
            {
                format!("{}://{}:{}@{}:{}", scheme, u, p, self.hostname, self.port)
            }
            _ => format!("{}://{}:{}", scheme, self.hostname, self.port),
        }
    }
}

// ── Choker algorithms ─────────────────────────────────────────────────

/// Choking algorithm used when we are seeding.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedChokingAlgorithm {
    /// Unchoke peers we upload to fastest.
    #[default]
    FastestUpload,
    /// Round-robin through all interested peers.
    RoundRobin,
    /// Prefer leechers over seeds (anti-leech).
    AntiLeech,
}

/// Top-level choking algorithm variant.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChokingAlgorithm {
    /// Fixed number of unchoke slots (libtorrent default).
    #[default]
    FixedSlots,
    /// Rate-based unchoking (auto-adjusts slots).
    RateBased,
}

// ── Mixed-mode (TCP/uTP) bandwidth ────────────────────────────────────

/// Mixed-mode bandwidth allocation algorithm for TCP/uTP coexistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MixedModeAlgorithm {
    /// Throttle uTP upload when any TCP peer is connected.
    /// uTP gets at most 10% of the global upload rate when TCP peers are present.
    PreferTcp,
    /// Allocate bandwidth proportional to the number of TCP vs uTP peers.
    PeerProportional,
}

// ── Alert category bitmask ────────────────────────────────────────────

bitflags::bitflags! {
    /// Bitmask categories for filtering alerts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AlertCategory: u32 {
        /// Torrent lifecycle: added, removed, paused, resumed, finished, state changes.
        const STATUS       = 0x001;
        /// Errors from torrents, trackers, storage.
        const ERROR        = 0x002;
        /// Peer connect/disconnect/ban events.
        const PEER         = 0x004;
        /// Tracker announce replies and errors.
        const TRACKER      = 0x008;
        /// Storage/file operations.
        const STORAGE      = 0x010;
        /// DHT bootstrap and peer discovery.
        const DHT          = 0x020;
        /// Periodic session/torrent statistics.
        const STATS        = 0x040;
        /// Piece-level events (verified, hash-failed).
        const PIECE        = 0x080;
        /// Block-level events (high volume).
        const BLOCK        = 0x100;
        /// Performance warnings.
        const PERFORMANCE  = 0x200;
        /// Port mapping (UPnP/NAT-PMP).
        const PORT_MAPPING = 0x400;
        /// I2P session events.
        const I2P          = 0x800;
        /// All categories enabled.
        const ALL          = 0xFFF;
    }
}
