use crate::krpc::Infohash;
use crate::storage::peer_outcomes::{PeerOutcome, PeerOutcomeWriter};
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use crate::verify::wire::{WireError, WireSession, gen_peer_id};
use librqbit_utp::UtpSocketUdp;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

const TCP_TIMEOUT: Duration = Duration::from_secs(5);
const UTP_TIMEOUT: Duration = Duration::from_secs(5);

const FAILED_PEER_SAMPLE_RATE: u64 = 500; // Log ~1 in 500 I/O failures (~0.2%)

fn sample_failed_peer(ih: &Infohash, addr: &SocketAddr, metrics: &Metrics) {
    let count = metrics.fetch_connect_io.load(Ordering::Relaxed);
    let hash = count
        .wrapping_mul(0x517c1b7275698a01)
        .wrapping_mul(u64::from(ih[0] as u64) | 1);
    if hash % FAILED_PEER_SAMPLE_RATE == 0 {
        tracing::warn!(
            addr = %addr,
            ih_prefix = ?&ih[..4],
            port = addr.port(),
            connect_io_count = count,
            "connect_failed_sample"
        );
    }
}

fn wire_error_to_outcome(e: &WireError) -> &'static str {
    match e {
        WireError::Io(_) => "metadata_eof",
        WireError::Timeout => "metadata_timeout",
        WireError::Handshake => "metadata_handshake",
        WireError::NoExtension => "metadata_no_extension",
        WireError::NoMetadataSize => "metadata_no_metadata_size",
        WireError::Eof => "metadata_eof",
        WireError::Cancelled => "metadata_cancelled",
        WireError::Reject => "metadata_reject",
        WireError::BadPiece => "metadata_bad_piece",
    }
}

enum FetchOutcome {
    ConnectFailed(SocketAddr, WireError),
    MetadataFailed(WireError),
    Success(Vec<u8>),
}

pub enum VerifyResult {
    Success(Vec<u8>),
    NoPeers,
    SourceTimeout,
    MetadataFailed,
}

enum SourceState {
    Peers,
    NoPeers,
    Timeout,
}

async fn try_fetch(
    addr: SocketAddr,
    ih: Infohash,
    pid: [u8; 20],
    metrics: Arc<Metrics>,
    cache: Arc<PeerCache>,
    utp: Option<Arc<UtpSocketUdp>>,
    peer_outcomes: Arc<PeerOutcomeWriter>,
    source: &str,
    fetch_timeout: Duration,
) -> FetchOutcome {
    metrics.tcp_attempts.add(1);

    let addr_str = addr.to_string();
    crate::trace_lifecycle!(&ih, "fetch_start", peer = addr_str.clone(), transport = "tcp");
    let start = std::time::Instant::now();

    let mut session = match WireSession::connect_tcp(addr, &ih, &pid, TCP_TIMEOUT).await {
        Ok(s) => {
            metrics.tcp_connect_ok.add(1);
            metrics.tcp_connect_actual.add(1);
            let result_str = "ok";
            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str.clone(), transport = "tcp", result = result_str, elapsed_ms = start.elapsed().as_millis() as u64);
            s
        }
        Err(tcp_err) => {
            let result_str = match &tcp_err {
                WireError::Timeout => "timeout",
                _ => "error",
            };
            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str.clone(), transport = "tcp", result = result_str, elapsed_ms = start.elapsed().as_millis() as u64);
            match &utp {
                Some(sock) => {
                    metrics.utp_attempts.add(1);
                    crate::trace_lifecycle!(&ih, "fetch_start", peer = addr_str.clone(), transport = "utp");
                    let utp_start = std::time::Instant::now();
                    match WireSession::connect_utp(sock.clone(), addr, &ih, &pid, UTP_TIMEOUT).await {
                        Ok(s) => {
                            metrics.utp_connect_ok.add(1);
                            metrics.utp_connect_actual.add(1);
                            let result_str = "ok";
                            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str.clone(), transport = "utp", result = result_str, elapsed_ms = utp_start.elapsed().as_millis() as u64);
                            s
                        }
                        Err(utp_err) => {
                            let result_str = match &utp_err {
                                WireError::Timeout => "timeout",
                                _ => "error",
                            };
                            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str.clone(), transport = "utp", result = result_str, elapsed_ms = utp_start.elapsed().as_millis() as u64);
                            cache.mark_bad(addr);
                            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: "tcp".to_string(), result: wire_error_to_outcome(&tcp_err).to_string(), client: None });
                            return FetchOutcome::ConnectFailed(addr, tcp_err);
                        }
                    }
                }
                None => {
                    cache.mark_bad(addr);
                    peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: "tcp".to_string(), result: wire_error_to_outcome(&tcp_err).to_string(), client: None });
                    return FetchOutcome::ConnectFailed(addr, tcp_err);
                }
            }
        }
    };

    let last_client = session.client().map(|s| s.to_string());
    match session.fetch_metadata(fetch_timeout).await {
        Ok(meta) => {
            if session.is_tcp() { metrics.tcp_metadata_ok.add(1); } else { metrics.utp_metadata_ok.add(1); }
            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: if session.is_tcp() { "tcp" } else { "utp" }.to_string(), result: "metadata_ok".to_string(), client: last_client });
            FetchOutcome::Success(meta)
        }
        Err(e) => {
            metrics.metadata_failed_io.add(1);
            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: if session.is_tcp() { "tcp" } else { "utp" }.to_string(), result: wire_error_to_outcome(&e).to_string(), client: last_client });
            FetchOutcome::MetadataFailed(e)
        }
    }
}

pub async fn verify_infohash(
    router: Arc<Router>,
    utp: Option<Arc<UtpSocketUdp>>,
    info_hash: Infohash,
    race_peers: usize,
    metrics: Arc<Metrics>,
    peer_cache: Arc<PeerCache>,
    announce_peer_cache: Arc<crate::verify::AnnouncePeerCache>,
    direct: Option<SocketAddr>,
    peer_outcomes: Arc<PeerOutcomeWriter>,
    fetch_timeout: Duration,
) -> VerifyResult {
    let (mut peers, state) = match source_peers(
        router,
        info_hash,
        race_peers.max(1),
        metrics.clone(),
    )
    .await
    {
        SourceResult::Peers(p) => (p, SourceState::Peers),
        SourceResult::NoPeers => (Vec::new(), SourceState::NoPeers),
        SourceResult::AllTimeout => (Vec::new(), SourceState::Timeout),
    };

    // Prepend the cached announcing peer if available (highest quality signal).
    let mut announce_peers = std::collections::HashSet::new();
    if let Some(announcer) = announce_peer_cache.get(&info_hash) {
        if !peers.contains(&announcer) {
            peers.insert(0, announcer);
            announce_peers.insert(announcer);
            crate::trace_lifecycle!(&info_hash, "announce_peer_injected", peer = announcer.to_string());
        }
    }

    if let Some(d) = direct {
        if !peers.contains(&d) {
            peers.insert(0, d);
            announce_peers.insert(d);
        }
    }
    peers.truncate(race_peers.max(1));
    if peers.is_empty() {
        return match state {
            SourceState::NoPeers => VerifyResult::NoPeers,
            _ => VerifyResult::SourceTimeout,
        };
    }

    metrics.fetch_attempts.add(peers.len() as u64);
    let peer_id = gen_peer_id();
    let mut set = JoinSet::new();
    for addr in peers {
        let ih = info_hash;
        let pid = peer_id;
        let metrics = metrics.clone();
        let cache = peer_cache.clone();
        let utp = utp.clone();
        let po = peer_outcomes.clone();
        let source = if announce_peers.contains(&addr) { "announce_peer" } else { "get_peers" };
        set.spawn(async move {
            try_fetch(addr, ih, pid, metrics, cache, utp, po, source, fetch_timeout).await
        });
    }

    let mut result = None;
    while let Some(res) = set.join_next().await {
        match res {
            Ok(FetchOutcome::Success(meta)) => {
                result = Some(meta);
                break;
            }
            Ok(FetchOutcome::ConnectFailed(_addr, WireError::Timeout)) => {
                metrics.fetch_connect_timeout.add(1);
                metrics.verify_timeouts.add(1);
            }
            Ok(FetchOutcome::ConnectFailed(_addr, WireError::Io(_))) => {
                metrics.fetch_connect_io.add(1);
                sample_failed_peer(&info_hash, &_addr, &metrics);
            }
            Ok(FetchOutcome::ConnectFailed(_addr, _)) => {
                metrics.fetch_connect_io.add(1);
                sample_failed_peer(&info_hash, &_addr, &metrics);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Timeout)) => {
                metrics.fetch_io.add(1);
                metrics.verify_timeouts.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Handshake)) => {
                metrics.fetch_handshake.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::NoExtension)) => {
                metrics.fetch_no_extension.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Reject)) => {
                metrics.fetch_reject.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::BadPiece)) => {
                metrics.fetch_bad_piece.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Io(_))) => {
                metrics.fetch_io.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Eof)) => {
                metrics.fetch_io.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::Cancelled)) => {
                metrics.fetch_io.add(1);
            }
            Ok(FetchOutcome::MetadataFailed(WireError::NoMetadataSize)) => {}
            Err(_) => {}
        }
    }
    set.abort_all();
    match result {
        Some(meta) => VerifyResult::Success(meta),
        None => VerifyResult::MetadataFailed,
    }
}
