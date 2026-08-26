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

#[derive(Clone)]
pub struct FetchParams {
    pub tcp_timeout: Duration,
    pub utp_timeout: Duration,
    pub metadata_timeout: Duration,
    pub source_deadline: Duration,
    pub source_k: usize,
    pub source_alpha: usize,
    pub source_query_timeout: Duration,
    pub source_max_queries: usize,
    pub race_peers: usize,
    pub failed_peer_sample_rate: u64,
}

fn sample_failed_peer(ih: &Infohash, addr: &SocketAddr, metrics: &Metrics, rate: u64) {
    let count = metrics.fetch_connect_io.load(Ordering::Relaxed);
    let hash = count
        .wrapping_mul(0x517c1b7275698a01)
        .wrapping_mul(u64::from(ih[0] as u64) | 1);
    if hash % rate == 0 {
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

fn connect_error_to_outcome(e: &WireError) -> &'static str {
    match e {
        WireError::Timeout => "timeout",
        WireError::Io(_) => "io_error",
        WireError::Handshake => "handshake",
        WireError::NoExtension => "no_extension",
        WireError::Eof => "io_error",
        WireError::Cancelled => "io_error",
        _ => "io_error",
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
    tcp_timeout: Duration,
    utp_timeout: Duration,
    limiter: Arc<crate::verify::ConnLimiter>,
) -> FetchOutcome {
    metrics.tcp_attempts.add(1);

    let addr_str = addr.to_string();
    crate::trace_lifecycle!(&ih, "fetch_start", stream = "fetch", peer = addr_str.clone(), transport = "tcp");
    let start = std::time::Instant::now();

    // Per-IP permit bounds only the connect+handshake phase. Releasing it
    // here (before metadata transfer) prevents a hot multi-port seedbox from
    // holding a permit up to the full 25s metadata timeout.
    let _permit = limiter.acquire(addr.ip()).await;

    let transport_str: &'static str;
    let mut session = match WireSession::connect_tcp(addr, &ih, &pid, tcp_timeout).await {
        Ok(s) => {
            metrics.tcp_connect_ok.add(1);
            metrics.tcp_connect_actual.add(1);
            crate::trace_lifecycle!(&ih, "connect_result", stream = "fetch", peer = addr_str.clone(), transport = "tcp", result = "ok", elapsed_ms = start.elapsed().as_millis() as u64);
            transport_str = "tcp";
            s
        }
        Err(tcp_err) => {
            crate::trace_lifecycle!(&ih, "connect_result", stream = "fetch", peer = addr_str.clone(), transport = "tcp", result = "error", elapsed_ms = start.elapsed().as_millis() as u64);
            match utp {
                Some(sock) => {
                    metrics.utp_attempts.add(1);
                    crate::trace_lifecycle!(&ih, "fetch_start", stream = "fetch", peer = addr_str.clone(), transport = "utp");
                    let utp_start = std::time::Instant::now();
                    match WireSession::connect_utp(sock, addr, &ih, &pid, utp_timeout).await {
                        Ok(s) => {
                            metrics.utp_connect_ok.add(1);
                            metrics.utp_connect_actual.add(1);
                            crate::trace_lifecycle!(&ih, "connect_result", stream = "fetch", peer = addr_str.clone(), transport = "utp", result = "ok", elapsed_ms = utp_start.elapsed().as_millis() as u64);
                            transport_str = "utp";
                            s
                        }
                        Err(utp_err) => {
                            let result_str = connect_error_to_outcome(&utp_err);
                            crate::trace_lifecycle!(&ih, "connect_result", stream = "fetch", peer = addr_str.clone(), transport = "utp", result = result_str, elapsed_ms = utp_start.elapsed().as_millis() as u64);
                            cache.mark_bad(addr);
                            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: "utp".to_string(), result: result_str.to_string(), client: None, phase: Some("connect".to_string()), elapsed_ms: Some(utp_start.elapsed().as_millis().min(i32::MAX as u128) as i32) });
                            return FetchOutcome::ConnectFailed(addr, utp_err);
                        }
                    }
                }
                None => {
                    cache.mark_bad(addr);
                    let result_str = connect_error_to_outcome(&tcp_err);
                    peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: "tcp".to_string(), result: result_str.to_string(), client: None, phase: Some("connect".to_string()), elapsed_ms: Some(start.elapsed().as_millis().min(i32::MAX as u128) as i32) });
                    return FetchOutcome::ConnectFailed(addr, tcp_err);
                }
            }
}
    };
    drop(_permit);

    let connect_elapsed = start.elapsed().as_millis().min(i32::MAX as u128) as i32;
    peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: transport_str.to_string(), result: "ok".to_string(), client: None, phase: Some("connect".to_string()), elapsed_ms: Some(connect_elapsed) });
    let last_client = session.client().map(|s| s.to_string());
    let metadata_start = std::time::Instant::now();
    match session.fetch_metadata(fetch_timeout).await {
        Ok(meta) => {
            if session.is_tcp() { metrics.tcp_metadata_ok.add(1); } else { metrics.utp_metadata_ok.add(1); }
            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: transport_str.to_string(), result: "metadata_ok".to_string(), client: last_client, phase: Some("metadata".to_string()), elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32) });
            FetchOutcome::Success(meta)
        }
        Err(e) => {
            metrics.metadata_failed_io.add(1);
            peer_outcomes.push(PeerOutcome { ih, peer: addr.to_string(), source: source.to_string(), transport: transport_str.to_string(), result: wire_error_to_outcome(&e).to_string(), client: last_client, phase: Some("metadata".to_string()), elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32) });
            FetchOutcome::MetadataFailed(e)
        }
    }
}

pub async fn verify_infohash(
    router: Arc<Router>,
    utp: Option<Arc<UtpSocketUdp>>,
    info_hash: Infohash,
    params: &FetchParams,
    metrics: Arc<Metrics>,
    peer_cache: Arc<PeerCache>,
    announce_peer_cache: Arc<crate::verify::AnnouncePeerCache>,
    direct: Option<SocketAddr>,
    peer_outcomes: Arc<PeerOutcomeWriter>,
    conn_limiter: Arc<crate::verify::ConnLimiter>,
) -> VerifyResult {
    let race_peers = params.race_peers.max(1);
    let peer_id = gen_peer_id();
    let fetch_timeout = params.metadata_timeout;
    let tcp_timeout = params.tcp_timeout;
    let utp_timeout = params.utp_timeout;

    let mut set: JoinSet<FetchOutcome> = JoinSet::new();
    let mut peers_seen: std::collections::HashSet<SocketAddr> = std::collections::HashSet::new();

    // 1) Immediately spawn the highest-quality leads: a cached announcing peer
    //    and/or the direct announce_peer. These are raced against the DHT
    //    lookup below so we never wait up to source_deadline behind querying
    //    mostly-dead DHT nodes for a peer that is live right now.
    if let Some(announcer) = announce_peer_cache.get(&info_hash) {
        if peers_seen.insert(announcer) {
            crate::trace_lifecycle!(&info_hash, "announce_peer_injected", stream = "fetch", peer = announcer.to_string());
        }
    }
    if let Some(d) = direct {
        peers_seen.insert(d);
    }
    for &addr in &peers_seen {
        let ih = info_hash;
        let pid = peer_id;
        let metrics = metrics.clone();
        let cache = peer_cache.clone();
        let utp = utp.clone();
        let po = peer_outcomes.clone();
        let limiter = conn_limiter.clone();
        metrics.fetch_attempts.add(1);
        set.spawn(async move {
            try_fetch(addr, ih, pid, metrics, cache, utp, po, "announce_peer", fetch_timeout, tcp_timeout, utp_timeout, limiter).await
        });
    }

    // 2) Spawn the DHT lookup concurrently so it never blocks the direct race.
    let router_clone = router.clone();
    let metrics_clone = metrics.clone();
    let source_deadline = params.source_deadline;
    let source_k = params.source_k;
    let source_alpha = params.source_alpha;
    let source_query_timeout = params.source_query_timeout;
    let source_max_queries = params.source_max_queries;
    let mut source_fut = tokio::spawn(async move {
        source_peers(
            router_clone,
            info_hash,
            race_peers,
            metrics_clone,
            source_deadline,
            source_k,
            source_alpha,
            source_query_timeout,
            source_max_queries,
        )
        .await
    });

    let mut source_state = SourceState::Timeout;
    let mut dht_done = false;
    let mut result = None;

    // 3) Race fetches against the DHT lookup; inject DHT-returned peers as they
    //    arrive, capped at race_peers total.
    loop {
        tokio::select! {
            res = set.join_next(), if !set.is_empty() => {
                match res {
                    Some(Ok(FetchOutcome::Success(meta))) => {
                        result = Some(meta);
                        break;
                    }
                    Some(Ok(outcome)) => {
                        match outcome {
                            FetchOutcome::ConnectFailed(_addr, WireError::Timeout) => {
                                metrics.fetch_connect_timeout.add(1);
                                metrics.verify_timeouts.add(1);
                            }
                            FetchOutcome::ConnectFailed(_addr, WireError::Io(_)) => {
                                metrics.fetch_connect_io.add(1);
                                sample_failed_peer(&info_hash, &_addr, &metrics, params.failed_peer_sample_rate.max(1));
                            }
                            FetchOutcome::ConnectFailed(_addr, _) => {
                                metrics.fetch_connect_io.add(1);
                                sample_failed_peer(&info_hash, &_addr, &metrics, params.failed_peer_sample_rate.max(1));
                            }
                            FetchOutcome::MetadataFailed(WireError::Timeout) => {
                                metrics.fetch_io.add(1);
                                metrics.verify_timeouts.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::Handshake) => {
                                metrics.fetch_handshake.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::NoExtension) => {
                                metrics.fetch_no_extension.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::Reject) => {
                                metrics.fetch_reject.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::BadPiece) => {
                                metrics.fetch_bad_piece.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::Io(_)) => {
                                metrics.fetch_io.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::Eof) => {
                                metrics.fetch_io.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::Cancelled) => {
                                metrics.fetch_io.add(1);
                            }
                            FetchOutcome::MetadataFailed(WireError::NoMetadataSize) => {}
                            FetchOutcome::Success(_) => {}
                        }
                    }
                    _ => {}
                }
            }
            dht_res = &mut source_fut, if !dht_done => {
                dht_done = true;
                if let Ok(lookup_res) = dht_res {
                    let (new_peers, state) = match lookup_res {
                        SourceResult::Peers(p) => (p, SourceState::Peers),
                        SourceResult::NoPeers => (Vec::new(), SourceState::NoPeers),
                        SourceResult::AllTimeout => (Vec::new(), SourceState::Timeout),
                    };
                    source_state = state;
                    for addr in new_peers {
                        if peers_seen.len() >= race_peers {
                            break;
                        }
                        if peers_seen.insert(addr) {
                            let ih = info_hash;
                            let pid = peer_id;
                            let metrics = metrics.clone();
                            let cache = peer_cache.clone();
                            let utp = utp.clone();
                            let po = peer_outcomes.clone();
                            let limiter = conn_limiter.clone();
                            metrics.fetch_attempts.add(1);
                            set.spawn(async move {
                                try_fetch(addr, ih, pid, metrics, cache, utp, po, "get_peers", fetch_timeout, tcp_timeout, utp_timeout, limiter).await
                            });
                        }
                    }
                }
            }
            else => {
                break;
            }
        }
    }

    set.abort_all();
    source_fut.abort();

    match result {
        Some(meta) => VerifyResult::Success(meta),
        None => {
            if peers_seen.is_empty() {
                match source_state {
                    SourceState::NoPeers => VerifyResult::NoPeers,
                    _ => VerifyResult::SourceTimeout,
                }
            } else {
                VerifyResult::MetadataFailed
            }
        }
    }
}
