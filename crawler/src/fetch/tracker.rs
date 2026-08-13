//! UDP tracker client (BEP 15) for live-peer resolution.
//!
//! DHT `get_peers` frequently returns empty because most sampled hashes are
//! dead. Trackers, by contrast, are *queried* by seeders/leechers of a given
//! torrent — a tracker response containing peers is strong evidence those
//! peers are alive. Querying a small set of well-known public trackers for a
//! hash that passed the liveness gate recovers a large share of the
//! `empty_peers` failures without making the crawler a visible DHT node.
//!
//! BEP 15: a connection-id handshake, then an announce with the 20-byte
//! infohash; the tracker replies with `compact` peer addresses.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use rand::RngCore;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Known public trackers that accept anonymous announces without a passkey.
/// A small spread is enough — each returns live peers for popular torrents.
const PUBLIC_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://tracker.openbittorrent.com:80/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.moeking.me:6969/announce",
];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(1);

const ACTION_CONNECT: i32 = 0;
const ACTION_ANNOUNCE: i32 = 1;
const EVENT_NONE: i32 = 0;

/// A connection-id handshake / announce exchange on a fresh UDP socket.
struct TrackerSocket {
    socket: UdpSocket,
}

impl TrackerSocket {
    async fn new(tracker: &SocketAddr) -> Result<Self> {
        let bind = if tracker.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).await?;
        socket.connect(tracker).await?;
        Ok(Self { socket })
    }

    /// BEP 15 connect: exchange a random transaction id for a connection id.
    async fn connect(&mut self) -> Result<(u64, u32)> {
        let mut req = Vec::with_capacity(16);
        req.extend_from_slice(&0x0417_2710_1980u64.to_be_bytes()); // protocol id
        req.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
        let txn = rand::thread_rng().next_u32();
        req.extend_from_slice(&txn.to_be_bytes());
        self.socket.send(&req).await?;

        let mut buf = [0u8; 2048];
        let n = timeout(CONNECT_TIMEOUT, self.socket.recv(&mut buf)).await??;
        if n < 16 {
            return Err(anyhow!("tracker connect response too short"));
        }
        let resp_txn = u32::from_be_bytes(buf[0..4].try_into().unwrap());
        let action = i32::from_be_bytes(buf[4..8].try_into().unwrap());
        if resp_txn != txn || action != ACTION_CONNECT {
            return Err(anyhow!("tracker connect rejected (action {action})"));
        }
        let conn_id = u64::from_be_bytes(buf[8..16].try_into().unwrap());
        Ok((conn_id, txn))
    }

    /// BEP 15 announce: ask for peers of `info_hash`. Returns compact peer
    /// addresses (6 bytes each for IPv4, 18 for IPv6).
    async fn announce(
        &mut self,
        conn_id: u64,
        connect_txn: u32,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        port: u16,
    ) -> Result<Vec<SocketAddr>> {
        let mut req = Vec::with_capacity(98);
        req.extend_from_slice(&conn_id.to_be_bytes());
        let txn = rand::thread_rng().next_u32();
        req.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
        req.extend_from_slice(&txn.to_be_bytes());
        req.extend_from_slice(info_hash);
        req.extend_from_slice(peer_id);
        req.extend_from_slice(&0i64.to_be_bytes()); // downloaded
        req.extend_from_slice(&0i64.to_be_bytes()); // left
        req.extend_from_slice(&0i64.to_be_bytes()); // uploaded
        req.extend_from_slice(&EVENT_NONE.to_be_bytes());
        req.extend_from_slice(&0u32.to_be_bytes()); // IP (0 = default)
        req.extend_from_slice(&connect_txn.to_be_bytes()); // key
        req.extend_from_slice(&(-1i32).to_be_bytes()); // num_want (default)
        req.extend_from_slice(&port.to_be_bytes());
        self.socket.send(&req).await?;

        let mut buf = [0u8; 65536];
        let n = timeout(ANNOUNCE_TIMEOUT, self.socket.recv(&mut buf)).await??;
        if n < 20 {
            return Err(anyhow!("tracker announce response too short"));
        }
        let resp_txn = u32::from_be_bytes(buf[0..4].try_into().unwrap());
        let action = i32::from_be_bytes(buf[4..8].try_into().unwrap());
        if resp_txn != txn {
            return Err(anyhow!("tracker announce transaction mismatch"));
        }
        if action != ACTION_ANNOUNCE {
            // A failure response carries an error message string.
            let msg = String::from_utf8_lossy(&buf[8..n]);
            return Err(anyhow!("tracker announce error: {msg}"));
        }
        // Offsets: 8 action, 12 txn, 16 interval, 20 leechers, 24 seeders,
        // 28..n compact peers.
        let mut peers = Vec::new();
        let peer_bytes = &buf[28..n];
        // Compact peers are 6-byte (v4) or 18-byte (v6) groups.
        let mut off = 0;
        while off + 6 <= peer_bytes.len() {
            let ip = std::net::Ipv4Addr::new(
                peer_bytes[off],
                peer_bytes[off + 1],
                peer_bytes[off + 2],
                peer_bytes[off + 3],
            );
            let port = u16::from_be_bytes([peer_bytes[off + 4], peer_bytes[off + 5]]);
            peers.push(SocketAddr::new(std::net::IpAddr::V4(ip), port));
            off += 6;
        }
        Ok(peers)
    }
}

/// Query a set of public trackers for live peers of `info_hash`, returning the
/// union of peer addresses (best-effort; tracker failures are skipped). All
/// trackers are queried CONCURRENTLY so the worst case is one tracker timeout,
/// not the sum of all timeouts — critical on the fetch hot path.
pub async fn resolve_peers_from_trackers(info_hash: &[u8; 20]) -> Vec<SocketAddr> {
    let mut peer_id = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut peer_id);
    let port = 6881u16;

    let mut tasks = tokio::task::JoinSet::new();
    for tracker_str in PUBLIC_TRACKERS {
        let info_hash = *info_hash;
        tasks.spawn(async move {
            let Ok(tracker) = parse_tracker(tracker_str).await else { return Vec::new() };
            query_tracker(tracker, &info_hash, &peer_id, port)
                .await
                .unwrap_or_default()
        });
    }
    let mut all = Vec::new();
    while let Some(peers) = tasks.join_next().await {
        if let Ok(peers) = peers {
            all.extend(peers);
        }
        if all.len() >= 200 {
            break;
        }
    }
    // Dedup.
    all.sort_unstable();
    all.dedup();
    all
}

/// Parse a `udp://host:port/announce` tracker URL into its socket address,
/// resolving the hostname via DNS (tracker hosts are usually hostnames).
async fn parse_tracker(url: &str) -> Result<SocketAddr> {
    let rest = url.strip_prefix("udp://").ok_or_else(|| anyhow!("not a udp tracker"))?;
    let host_port = rest.split('/').next().unwrap_or(rest);
    // Prefer an IPv4 literal / resolved address for the UDP tracker socket.
    let mut addrs = tokio::net::lookup_host(host_port).await?;
    let mut best: Option<SocketAddr> = None;
    for a in &mut addrs {
        if a.is_ipv4() {
            return Ok(a);
        }
        best.get_or_insert(a);
    }
    best.ok_or_else(|| anyhow!("tracker host resolved to no addresses"))
}

async fn query_tracker(
    tracker: SocketAddr,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    port: u16,
) -> Result<Vec<SocketAddr>> {
    let mut sock = TrackerSocket::new(&tracker).await?;
    let (conn_id, connect_txn) = sock.connect().await?;
    sock.announce(conn_id, connect_txn, info_hash, peer_id, port).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_udp_tracker_url() {
        // Hostname trackers resolve to a socket addr (not a literal compare).
        let a = parse_tracker("udp://tracker.opentrackr.org:1337/announce").await.unwrap();
        assert_eq!(a.port(), 1337);
        assert!(a.ip().is_ipv4() || a.ip().is_ipv6());
        let b = parse_tracker("udp://open.demonii.com:1337/announce").await.unwrap();
        assert_eq!(b.port(), 1337);
    }

    #[tokio::test]
    async fn parses_ip_tracker_url() {
        let a = parse_tracker("udp://1.2.3.4:6969/announce").await.unwrap();
        assert_eq!(a, "1.2.3.4:6969".parse::<SocketAddr>().unwrap());
    }

    #[tokio::test]
    async fn parses_ipv6_tracker_url() {
        let a = parse_tracker("udp://[::1]:6969/announce").await.unwrap();
        assert_eq!(a, "[::1]:6969".parse().unwrap());
    }

    #[tokio::test]
    async fn rejects_non_udp_tracker() {
        assert!(parse_tracker("http://tracker.example.com/announce").await.is_err());
    }
}
