use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use crate::verify::wire::{WireError, WireSession, gen_peer_id};
use librqbit_utp::UtpSocketUdp;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const TCP_TIMEOUT: Duration = Duration::from_secs(2);
const UTP_TIMEOUT: Duration = Duration::from_secs(4);
const FETCH_TIMEOUT: Duration = Duration::from_secs(25);

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
) -> FetchOutcome {
    metrics.tcp_attempts.add(1);
    if utp.is_some() {
        metrics.utp_attempts.add(1);
    }

    let addr_str = addr.to_string();

    let mut tcp_task = tokio::spawn({
        let ih = ih;
        let pid = pid;
        let metrics = metrics.clone();
        let addr_str = addr_str.clone();
        crate::trace_lifecycle!(&ih, "fetch_start", peer = addr_str.clone(), transport = "tcp");
        let start = std::time::Instant::now();
        async move {
            let res = WireSession::connect_tcp(addr, &ih, &pid, TCP_TIMEOUT).await;
            if res.is_ok() {
                metrics.tcp_connect_actual.add(1);
            }
            let result_str = match &res {
                Ok(_) => "ok",
                Err(WireError::Timeout) => "timeout",
                Err(_) => "error",
            };
            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str, transport = "tcp", result = result_str, elapsed_ms = start.elapsed().as_millis() as u64);
            res
        }
    });
    let mut utp_task = tokio::spawn({
        let ih = ih;
        let pid = pid;
        let utp = utp.clone();
        let metrics = metrics.clone();
        let addr_str = addr_str.clone();
        crate::trace_lifecycle!(&ih, "fetch_start", peer = addr_str.clone(), transport = "utp");
        let start = std::time::Instant::now();
        async move {
            let res = match utp {
                Some(sock) => {
                    WireSession::connect_utp(sock, addr, &ih, &pid, UTP_TIMEOUT).await
                }
                None => Err(WireError::Io(std::io::Error::other("no uTP socket"))),
            };
            if res.is_ok() {
                metrics.utp_connect_actual.add(1);
            }
            let result_str = match &res {
                Ok(_) => "ok",
                Err(WireError::Timeout) => "timeout",
                Err(_) => "error",
            };
            crate::trace_lifecycle!(&ih, "connect_result", peer = addr_str, transport = "utp", result = result_str, elapsed_ms = start.elapsed().as_millis() as u64);
            res
        }
    });

    let (first_session, first_is_tcp, loser_handle) = tokio::select! {
        res = &mut tcp_task => {
            match res {
                Ok(Ok(s)) => { metrics.tcp_connect_ok.add(1); (Ok(s), true, utp_task) }
                Ok(Err(e)) => {
                    let utp_res = utp_task.await;
                    match utp_res {
                        Ok(Ok(s)) => { metrics.utp_connect_ok.add(1); (Ok(s), false, tcp_task) }
                        Ok(Err(utp_e)) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                        Err(_) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                    }
                }
                Err(_) => {
                    let utp_res = utp_task.await;
                    match utp_res {
                        Ok(Ok(s)) => { metrics.utp_connect_ok.add(1); (Ok(s), false, tcp_task) }
                        Ok(Err(e)) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                        Err(_) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, WireError::Io(std::io::Error::other("both tasks failed")));
                        }
                    }
                }
            }
        }
        res = &mut utp_task => {
            match res {
                Ok(Ok(s)) => { metrics.utp_connect_ok.add(1); (Ok(s), false, tcp_task) }
                Ok(Err(e)) => {
                    let tcp_res = tcp_task.await;
                    match tcp_res {
                        Ok(Ok(s)) => { metrics.tcp_connect_ok.add(1); (Ok(s), true, utp_task) }
                        Ok(Err(tcp_e)) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                        Err(_) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                    }
                }
                Err(_) => {
                    let tcp_res = tcp_task.await;
                    match tcp_res {
                        Ok(Ok(s)) => { metrics.tcp_connect_ok.add(1); (Ok(s), true, utp_task) }
                        Ok(Err(e)) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, e);
                        }
                        Err(_) => {
                            cache.mark_bad(addr);
                            return FetchOutcome::ConnectFailed(addr, WireError::Io(std::io::Error::other("both tasks failed")));
                        }
                    }
                }
            }
        }
    };

    let mut session = match first_session {
        Ok(s) => s,
        Err(e) => {
            cache.mark_bad(addr);
            return FetchOutcome::ConnectFailed(addr, e);
        }
    };

    match session.fetch_metadata(FETCH_TIMEOUT).await {
        Ok(meta) => {
            loser_handle.abort();
            if first_is_tcp { metrics.tcp_metadata_ok.add(1); } else { metrics.utp_metadata_ok.add(1); }
            FetchOutcome::Success(meta)
        }
        Err(first_err) => {
            let loser_result = loser_handle.await;
            match loser_result {
                Ok(Ok(mut loser_session)) => {
                    match loser_session.fetch_metadata(FETCH_TIMEOUT).await {
                        Ok(meta) => {
                            if loser_session.is_tcp() { metrics.tcp_metadata_ok.add(1); } else { metrics.utp_metadata_ok.add(1); }
                            FetchOutcome::Success(meta)
                        }
                        Err(_) => FetchOutcome::MetadataFailed(first_err),
                    }
                }
                _ => FetchOutcome::MetadataFailed(first_err),
            }
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
    direct: Option<SocketAddr>,
) -> VerifyResult {
    let (mut peers, state) = match source_peers(
        router,
        info_hash,
        race_peers.max(1),
        metrics.clone(),
        peer_cache.clone(),
    )
    .await
    {
        SourceResult::Peers(p) => (p, SourceState::Peers),
        SourceResult::NoPeers => (Vec::new(), SourceState::NoPeers),
        SourceResult::AllTimeout => (Vec::new(), SourceState::Timeout),
    };
    if let Some(d) = direct {
        if !peers.contains(&d) {
            peers.insert(0, d);
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
        set.spawn(async move {
            try_fetch(addr, ih, pid, metrics, cache, utp).await
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
            }
            Ok(FetchOutcome::ConnectFailed(_, _)) => {
                metrics.fetch_connect_io.add(1);
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
            Ok(FetchOutcome::MetadataFailed(WireError::NoMetadata)) => {}
            Err(_) => {}
        }
    }
    set.abort_all();
    match result {
        Some(meta) => VerifyResult::Success(meta),
        None => VerifyResult::MetadataFailed,
    }
}
