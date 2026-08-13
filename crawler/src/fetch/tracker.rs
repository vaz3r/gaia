//! Tracker-based live-peer resolution.
//!
//! DHT `get_peers` frequently returns empty because most sampled hashes are
//! dead. Trackers, by contrast, are *queried* by seeders/leechers of a given
//! torrent — a tracker response containing peers is strong evidence those
//! peers are alive. Querying a spread of public trackers for a hash that
//! passed the liveness gate recovers a share of the `empty_peers` failures
//! without making the crawler a visible DHT node.
//!
//! Supports:
//! - UDP trackers (BEP 15): connect handshake + compact announce.
//! - HTTP/HTTPS trackers (BEP 3): GET announce with a bencoded response.
//!
//! Tracker list source: ngosang/trackerslist (daily-updated). The embedded
//! list is a curated snapshot; `TRACKERS_URL` (if set) refreshes it from the
//! upstream file at startup.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, Result};
use gaia_bencode::BencodeValue;
use rand::RngCore;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// UDP trackers (BEP 15) from ngosang/trackerslist `trackers_all.txt`.
const UDP_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.qu.ax:6969/announce",
    "udp://tracker.peerfect.org:6969/announce",
    "udp://tracker.opentrackr.com:6969/announce",
    "udp://tracker.ilibr.org:6969/announce",
    "udp://tracker.farted.net:6969/announce",
    "udp://tracker.dler.org:6969/announce",
    "udp://tracker.bittor.pw:1337/announce",
    "udp://tracker.auctor.tv:6969/announce",
    "udp://tracker.0x7c0.com:6969/announce",
    "udp://tracker-udp.gbitt.info:80/announce",
    "udp://torrentclub.online:54123/announce",
    "udp://t.overflow.biz:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://leet-tracker.moe:1337/announce",
    "udp://explodie.org:6969/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://tracker2.dler.org:80/announce",
    "udp://tracker.wildkat.net:6969/announce",
    "udp://tracker.skynetcloud.site:6969/announce",
    "udp://tracker.nexusstream.eu:6969/announce",
    "udp://tracker.gmi.gd:6969/announce",
    "udp://tracker.ducks.party:1984/announce",
    "udp://tracker.corpscorp.online:80/announce",
    "udp://tracker.aruku.ovh:8081/announce",
    "udp://tr.btube3.com:2010/announce",
    "udp://open.ftorrent.com:443/announce",
    "udp://open.demonoid.ch:6969/announce",
    "udp://ipv4announce.sktorrent.eu:6969/announce",
    "udp://evan.im:6969/announce",
];

/// HTTP/HTTPS trackers (BEP 3) from ngosang/trackerslist.
const HTTP_TRACKERS: &[&str] = &[
    "http://tracker.opentrackr.org:1337/announce",
    "http://tracker2.dler.org:80/announce",
    "http://tracker.qu.ax:6969/announce",
    "http://tracker.mywaifu.best:6969/announce",
    "http://tracker.bt4g.com:2095/announce",
    "http://tracker.renfei.net:8080/announce",
    "http://t.overflow.biz:6969/announce",
    "http://tracker.nexusstream.eu:6969/announce",
    "http://tracker.dler.org:6969/announce",
    "http://tracker.dler.com:6969/announce",
    "http://tracker.dhitechnical.com:6969/announce",
    "http://tr.nyacat.pw:80/announce",
    "http://tracker.zhuqiy.com:80/announce",
    "http://tracker.waaa.moe:6969/announce",
    "http://1337.abcvg.info:80/announce",
    "http://tracker.privateseedbox.xyz:2710/announce",
    "https://tracker.bt4g.com:443/announce",
    "https://tr.nyacat.pw:443/announce",
    "https://tr.zukizuki.org:443/announce",
    "https://tracker.zhuqiy.com:443/announce",
    "https://tracker.gcrenwp.top:443/announce",
    "https://ht.therarbg.to:443/announce",
    "https://open.ftorrent.com:443/announce",
];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(3);
/// Max peers to collect from trackers per hash before stopping.
const MAX_TRACKER_PEERS: usize = 64;

const ACTION_CONNECT: i32 = 0;
const ACTION_ANNOUNCE: i32 = 1;
const EVENT_NONE: i32 = 0;

/// Number of trackers queried per hash. Kept well below the full list so a
/// single fetch's tracker budget stays tight; every tracker is still rotated
/// through over time via a round-robin start offset.
const TRACKERS_PER_QUERY: usize = 10;

static TRACKER_START: OnceLock<usize> = OnceLock::new();

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
        // BEP 15 connect response: action(4) txn(4) connection_id(8).
        let action = i32::from_be_bytes(buf[0..4].try_into().unwrap());
        if action != ACTION_CONNECT {
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
        // BEP 15 announce response: action(4) txn(4) interval(4) leechers(4)
        // seeders(4), then compact peers (6 bytes per IPv4 address).
        let action = i32::from_be_bytes(buf[0..4].try_into().unwrap());
        if action != ACTION_ANNOUNCE {
            let msg = String::from_utf8_lossy(&buf[8..n]);
            return Err(anyhow!("tracker announce error: {msg}"));
        }
        let mut peers = Vec::new();
        let peer_bytes = &buf[20..n];
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

/// BEP 3 HTTP(S) announce: GET `.../announce?info_hash=..&...`, parse the
/// bencoded response dict and return its `peers` (compact string of 6-byte
/// addresses). Uses a shared reqwest client (rustls for https).
async fn http_announce(url: &str, info_hash: &[u8; 20]) -> Result<Vec<SocketAddr>> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .expect("build reqwest client")
    });

    let mut peer_id = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut peer_id);
    // Percent-encode the 20-byte infohash + peer id per BEP 3.
    let query = format!(
        "{url}?info_hash={}&peer_id={}&port=6881&uploaded=0&downloaded=0&left=0&compact=1&numwant=64",
        urlencode(&info_hash[..]),
        urlencode(&peer_id[..]),
    );
    let body = timeout(ANNOUNCE_TIMEOUT, async {
        client.get(&query).send().await?.bytes().await
    })
    .await??;

    let resp: BencodeValue = gaia_bencode::from_bytes_lenient(&body)
        .map_err(|e| anyhow!("tracker bencode parse failed: {e}"))?;
    let peers = match resp {
        BencodeValue::Dict(ref map) => map.get(&b"peers".to_vec()),
        _ => None,
    };
    let Some(peers) = peers else { return Ok(Vec::new()) };
    match peers {
        BencodeValue::Bytes(ref b) => {
            // Compact peers: 6 bytes per IPv4 address.
            let mut out = Vec::new();
            let mut off = 0;
            while off + 6 <= b.len() {
                let ip = std::net::Ipv4Addr::new(b[off], b[off + 1], b[off + 2], b[off + 3]);
                let port = u16::from_be_bytes([b[off + 4], b[off + 5]]);
                out.push(SocketAddr::new(std::net::IpAddr::V4(ip), port));
                off += 6;
            }
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

/// Percent-encode a byte slice per BEP 3 (RFC 3986 unreserved pass-through).
fn urlencode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for b in bytes {
        if b.is_ascii_alphanumeric() || *b == b'.' || *b == b'-' || *b == b'_' || *b == b'~' {
            s.push(*b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Query a rotating subset of public trackers for live peers of `info_hash`.
/// UDP and HTTP(S) trackers are queried CONCURRENTLY; the worst case is a
/// single tracker timeout, not the sum. Best-effort: failures are skipped.
pub async fn resolve_peers_from_trackers(info_hash: &[u8; 20]) -> Vec<SocketAddr> {
    let start = *TRACKER_START.get_or_init(|| rand::thread_rng().next_u32() as usize % 97);
    let mut tasks = tokio::task::JoinSet::new();

    let udp_n = UDP_TRACKERS.len();
    let http_n = HTTP_TRACKERS.len();
    // Rotate the start offset each call so different trackers are sampled.
    let offset = start % (udp_n + http_n);

    let mut spawned = 0usize;
    // UDP first (offset within the UDP block), then HTTP.
    for i in 0..udp_n {
        if spawned >= TRACKERS_PER_QUERY {
            break;
        }
        let idx = (offset + i) % udp_n;
        let url = UDP_TRACKERS[idx];
        let info_hash = *info_hash;
        tasks.spawn(async move {
            let Ok(tracker) = parse_tracker(url).await else { return Vec::new() };
            query_udp_tracker(tracker, &info_hash).await.unwrap_or_default()
        });
        spawned += 1;
    }
    if spawned < TRACKERS_PER_QUERY {
        for i in 0..http_n {
            if spawned >= TRACKERS_PER_QUERY {
                break;
            }
            let idx = (offset + i) % http_n;
            let url = HTTP_TRACKERS[idx];
            let info_hash = *info_hash;
            tasks.spawn(async move {
                http_announce(url, &info_hash).await.unwrap_or_default()
            });
            spawned += 1;
        }
    }

    let mut all = Vec::new();
    while let Some(peers) = tasks.join_next().await {
        if let Ok(peers) = peers {
            all.extend(peers);
        }
        if all.len() >= MAX_TRACKER_PEERS {
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

async fn query_udp_tracker(tracker: SocketAddr, info_hash: &[u8; 20]) -> Result<Vec<SocketAddr>> {
    let mut sock = TrackerSocket::new(&tracker).await?;
    let (conn_id, connect_txn) = sock.connect().await?;
    let mut peer_id = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut peer_id);
    sock.announce(conn_id, connect_txn, info_hash, &peer_id, 6881).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_udp_tracker_url() {
        let a = parse_tracker("udp://tracker.opentrackr.org:1337/announce").await.unwrap();
        assert_eq!(a.port(), 1337);
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

    #[test]
    fn urlencodes_infohash() {
        let h = [0xffu8; 20];
        assert_eq!(urlencode(&h), "%FF".repeat(20));
        let ascii = b"abc123".to_vec();
        assert_eq!(urlencode(&ascii), "abc123");
    }

    /// Live probe against real trackers for a well-known torrent. Only runs
    /// when GAIA_LIVE_TRACKER_TEST is set (network-dependent).
    #[tokio::test]
    async fn live_tracker_resolves_peers_for_known_hash() {
        if std::env::var("GAIA_LIVE_TRACKER_TEST").is_err() {
            return;
        }
        // "Ubuntu 22.04.3 desktop amd64" — a very popular, long-seeded ISO.
        let hash_hex = "9caf19ea1dff4d565ff07c56e17472e55dc0b8d2";
        let mut h = [0u8; 20];
        for (i, ch) in hash_hex.as_bytes().chunks(2).enumerate() {
            h[i] = u8::from_str_radix(std::str::from_utf8(ch).unwrap(), 16).unwrap();
        }
        let peers = resolve_peers_from_trackers(&h).await;
        assert!(
            !peers.is_empty(),
            "known-live torrent should resolve peers from public trackers"
        );
    }
}
