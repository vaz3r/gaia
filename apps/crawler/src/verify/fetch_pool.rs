use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::storage::peer_outcomes::{PeerOutcome, PeerOutcomeWriter};
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use crate::verify::wire::{WireError, WireSession, gen_peer_id};
use librqbit_utp::UtpSocketUdp;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
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
    pub transport_race_concurrent: bool,
    pub connect_deadline: Duration,
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

fn saturating_add_atomic(target: &std::sync::atomic::AtomicU64, delta: u64) {
    let mut prev = target.load(Ordering::Relaxed);
    loop {
        let next = prev.saturating_add(delta);
        match target.compare_exchange_weak(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => prev = actual,
        }
    }
}

struct FetchActiveGuard {
    metrics: Arc<Metrics>,
}
impl FetchActiveGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        metrics.fetch_active.fetch_add(1, Ordering::Relaxed);
        FetchActiveGuard { metrics }
    }
}
impl Drop for FetchActiveGuard {
    fn drop(&mut self) {
        let prev = self.metrics.fetch_active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(prev > 0, "fetch_active underflow");
    }
}

struct MetadataActiveGuard {
    metrics: Arc<Metrics>,
}
impl MetadataActiveGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        metrics.metadata_active.fetch_add(1, Ordering::Relaxed);
        MetadataActiveGuard { metrics }
    }
}
impl Drop for MetadataActiveGuard {
    fn drop(&mut self) {
        let prev = self.metrics.metadata_active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(prev > 0, "metadata_active underflow");
    }
}

#[allow(dead_code)]
struct SourceActiveGuard {
    metrics: Arc<Metrics>,
}
#[allow(dead_code)]
impl SourceActiveGuard {
    fn new(metrics: Arc<Metrics>) -> Self {
        metrics.source_active.fetch_add(1, Ordering::Relaxed);
        SourceActiveGuard { metrics }
    }
}
impl Drop for SourceActiveGuard {
    fn drop(&mut self) {
        let prev = self.metrics.source_active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(prev > 0, "source_active underflow");
    }
}

#[derive(Debug, PartialEq)]
pub enum Transport {
    Tcp,
    Utp,
}

/// Race two connection futures concurrently under an outer deadline.
/// Returns the first success, or the error from the last transport to fail
/// if both fail. `on_result` is called for each transport that actually
/// completes (not cancelled by a win), with `(transport, outcome, elapsed_ms)`.
async fn race_transports<S: Send>(
    tcp_fut: impl std::future::Future<Output = Result<S, WireError>> + Send,
    utp_fut: impl std::future::Future<Output = Result<S, WireError>> + Send,
    deadline: Duration,
    mut on_result: impl FnMut(Transport, &'static str, u64),
) -> Result<(Transport, S), WireError> {
    tokio::pin!(tcp_fut);
    tokio::pin!(utp_fut);
    let mut tcp_done = false;
    let mut utp_done = false;
    let mut last_err = None;
    let start = std::time::Instant::now();

    tokio::time::timeout(deadline, async {
        loop {
            tokio::select! {
                res = &mut tcp_fut, if !tcp_done => {
                    tcp_done = true;
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    match res {
                        Ok(s) => {
                            on_result(Transport::Tcp, "ok", elapsed_ms);
                            break Ok((Transport::Tcp, s));
                        }
                        Err(e) => {
                            on_result(Transport::Tcp, connect_error_to_outcome(&e), elapsed_ms);
                            if utp_done { break Err(e); }
                            last_err = Some(e);
                        }
                    }
                }
                res = &mut utp_fut, if !utp_done => {
                    utp_done = true;
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    match res {
                        Ok(s) => {
                            on_result(Transport::Utp, "ok", elapsed_ms);
                            break Ok((Transport::Utp, s));
                        }
                        Err(e) => {
                            on_result(Transport::Utp, connect_error_to_outcome(&e), elapsed_ms);
                            if tcp_done { break Err(last_err.unwrap_or(e)); }
                        }
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| WireError::Timeout)?
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
    transport_race_concurrent: bool,
    connect_deadline: Duration,
) -> FetchOutcome {
    let _fetch_guard = FetchActiveGuard::new(metrics.clone());
    metrics.tcp_attempts.add(1);

    let addr_str = addr.to_string();
    crate::trace_lifecycle!(
        &ih,
        "fetch_start",
        stream = "fetch",
        peer = addr_str.clone(),
        transport = "tcp"
    );
    let start = std::time::Instant::now();

    // Per-IP permit bounds only the connect+handshake phase. Releasing it
    // here (before metadata transfer) prevents a hot multi-port seedbox from
    // holding a permit up to the full 25s metadata timeout.
    let per_ip_start = Instant::now();
    let _permit = limiter.acquire(addr.ip()).await;
    let per_ip_us = per_ip_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.per_ip_wait_micros_total, per_ip_us);
    metrics
        .per_ip_acquisitions_total
        .fetch_add(1, Ordering::Relaxed);

    let transport_start = Instant::now();
    let transport_str: &'static str;
    let mut session = if transport_race_concurrent && utp.is_some() {
        metrics.utp_attempts.add(1);
        crate::trace_lifecycle!(
            &ih,
            "fetch_start",
            stream = "fetch",
            peer = addr_str.clone(),
            transport = "utp"
        );

        let tcp_fut = WireSession::connect_tcp(addr, &ih, &pid, tcp_timeout);
        let utp_sock = utp.clone().unwrap();
        let utp_fut = WireSession::connect_utp(utp_sock, addr, &ih, &pid, utp_timeout);

        let po = peer_outcomes.clone();
        let ih_clone = ih;
        let addr_clone = addr;
        let source_clone = source.to_string();
        let addr_str_clone = addr_str.clone();

        match race_transports(
            tcp_fut,
            utp_fut,
            connect_deadline,
            move |transport, result, elapsed_ms| {
                let transport_str = match transport {
                    Transport::Tcp => "tcp",
                    Transport::Utp => "utp",
                };
                crate::trace_lifecycle!(
                    &ih_clone,
                    "connect_result",
                    stream = "fetch",
                    peer = addr_str_clone.clone(),
                    transport = transport_str,
                    result = result,
                    elapsed_ms = elapsed_ms
                );
                po.push(PeerOutcome {
                    ih: ih_clone,
                    peer: addr_clone.to_string(),
                    source: source_clone.clone(),
                    transport: transport_str.to_string(),
                    result: result.to_string(),
                    client: None,
                    phase: Some("connect".to_string()),
                    elapsed_ms: Some(elapsed_ms.min(i32::MAX as u64) as i32),
                });
            },
        )
        .await
        {
            Ok((Transport::Tcp, s)) => {
                metrics.tcp_connect_ok.add(1);
                metrics.tcp_connect_actual.add(1);
                transport_str = "tcp";
                s
            }
            Ok((Transport::Utp, s)) => {
                metrics.utp_connect_ok.add(1);
                metrics.utp_connect_actual.add(1);
                transport_str = "utp";
                s
            }
            Err(e) => {
                cache.mark_bad(addr);
                let transport_us =
                    transport_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
                saturating_add_atomic(&metrics.transport_connect_micros_total, transport_us);
                metrics
                    .transport_connect_completed_total
                    .fetch_add(1, Ordering::Relaxed);
                return FetchOutcome::ConnectFailed(addr, e);
            }
        }
    } else {
        // Sequential path: TCP first, uTP fallback on failure (original logic).
        match WireSession::connect_tcp(addr, &ih, &pid, tcp_timeout).await {
            Ok(s) => {
                metrics.tcp_connect_ok.add(1);
                metrics.tcp_connect_actual.add(1);
                crate::trace_lifecycle!(
                    &ih,
                    "connect_result",
                    stream = "fetch",
                    peer = addr_str.clone(),
                    transport = "tcp",
                    result = "ok",
                    elapsed_ms = start.elapsed().as_millis() as u64
                );
                transport_str = "tcp";
                s
            }
            Err(tcp_err) => {
                crate::trace_lifecycle!(
                    &ih,
                    "connect_result",
                    stream = "fetch",
                    peer = addr_str.clone(),
                    transport = "tcp",
                    result = "error",
                    elapsed_ms = start.elapsed().as_millis() as u64
                );
                let result_str = connect_error_to_outcome(&tcp_err);
                peer_outcomes.push(PeerOutcome {
                    ih,
                    peer: addr.to_string(),
                    source: source.to_string(),
                    transport: "tcp".to_string(),
                    result: result_str.to_string(),
                    client: None,
                    phase: Some("connect".to_string()),
                    elapsed_ms: Some(start.elapsed().as_millis().min(i32::MAX as u128) as i32),
                });
                match utp {
                    Some(sock) => {
                        metrics.utp_attempts.add(1);
                        crate::trace_lifecycle!(
                            &ih,
                            "fetch_start",
                            stream = "fetch",
                            peer = addr_str.clone(),
                            transport = "utp"
                        );
                        let utp_start = std::time::Instant::now();
                        match WireSession::connect_utp(sock, addr, &ih, &pid, utp_timeout).await {
                            Ok(s) => {
                                metrics.utp_connect_ok.add(1);
                                metrics.utp_connect_actual.add(1);
                                crate::trace_lifecycle!(
                                    &ih,
                                    "connect_result",
                                    stream = "fetch",
                                    peer = addr_str.clone(),
                                    transport = "utp",
                                    result = "ok",
                                    elapsed_ms = utp_start.elapsed().as_millis() as u64
                                );
                                transport_str = "utp";
                                s
                            }
                            Err(utp_err) => {
                                let result_str = connect_error_to_outcome(&utp_err);
                                crate::trace_lifecycle!(
                                    &ih,
                                    "connect_result",
                                    stream = "fetch",
                                    peer = addr_str.clone(),
                                    transport = "utp",
                                    result = result_str,
                                    elapsed_ms = utp_start.elapsed().as_millis() as u64
                                );
                                peer_outcomes.push(PeerOutcome {
                                    ih,
                                    peer: addr.to_string(),
                                    source: source.to_string(),
                                    transport: "utp".to_string(),
                                    result: result_str.to_string(),
                                    client: None,
                                    phase: Some("connect".to_string()),
                                    elapsed_ms: Some(
                                        utp_start.elapsed().as_millis().min(i32::MAX as u128)
                                            as i32,
                                    ),
                                });
                                cache.mark_bad(addr);
                                let transport_us =
                                    transport_start.elapsed().as_micros().min(u64::MAX as u128)
                                        as u64;
                                saturating_add_atomic(
                                    &metrics.transport_connect_micros_total,
                                    transport_us,
                                );
                                metrics
                                    .transport_connect_completed_total
                                    .fetch_add(1, Ordering::Relaxed);
                                return FetchOutcome::ConnectFailed(addr, utp_err);
                            }
                        }
                    }
                    None => {
                        cache.mark_bad(addr);
                        let transport_us =
                            transport_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
                        saturating_add_atomic(
                            &metrics.transport_connect_micros_total,
                            transport_us,
                        );
                        metrics
                            .transport_connect_completed_total
                            .fetch_add(1, Ordering::Relaxed);
                        return FetchOutcome::ConnectFailed(addr, tcp_err);
                    }
                }
            }
        }
    };
    let transport_us = transport_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.transport_connect_micros_total, transport_us);
    metrics
        .transport_connect_completed_total
        .fetch_add(1, Ordering::Relaxed);
    drop(_permit);

    let connect_elapsed = start.elapsed().as_millis().min(i32::MAX as u128) as i32;
    peer_outcomes.push(PeerOutcome {
        ih,
        peer: addr.to_string(),
        source: source.to_string(),
        transport: transport_str.to_string(),
        result: "ok".to_string(),
        client: None,
        phase: Some("connect".to_string()),
        elapsed_ms: Some(connect_elapsed),
    });
    let last_client = session.client().map(|s| s.to_string());
    let _meta_guard = MetadataActiveGuard::new(metrics.clone());
    let metadata_start = Instant::now();
    let outcome = match session.fetch_metadata(fetch_timeout).await {
        Ok(meta) => {
            if session.is_tcp() {
                metrics.tcp_metadata_ok.add(1);
            } else {
                metrics.utp_metadata_ok.add(1);
            }
            peer_outcomes.push(PeerOutcome {
                ih,
                peer: addr.to_string(),
                source: source.to_string(),
                transport: transport_str.to_string(),
                result: "metadata_ok".to_string(),
                client: last_client,
                phase: Some("metadata".to_string()),
                elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32),
            });
            FetchOutcome::Success(meta)
        }
        Err(e) => {
            metrics.metadata_failed_io.add(1);
            peer_outcomes.push(PeerOutcome {
                ih,
                peer: addr.to_string(),
                source: source.to_string(),
                transport: transport_str.to_string(),
                result: wire_error_to_outcome(&e).to_string(),
                client: last_client,
                phase: Some("metadata".to_string()),
                elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32),
            });
            FetchOutcome::MetadataFailed(e)
        }
    };
    let meta_us = metadata_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.metadata_exchange_micros_total, meta_us);
    metrics
        .metadata_exchange_completed_total
        .fetch_add(1, Ordering::Relaxed);
    outcome
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
    fetch_limit: Arc<Semaphore>,
) -> VerifyResult {
    let race_peers = params.race_peers.max(1);
    let peer_id = gen_peer_id();
    let fetch_timeout = params.metadata_timeout;
    let tcp_timeout = params.tcp_timeout;
    let utp_timeout = params.utp_timeout;
    let transport_race_concurrent = params.transport_race_concurrent;
    let connect_deadline = params.connect_deadline;

    let mut set: JoinSet<FetchOutcome> = JoinSet::new();
    let mut peers_seen: std::collections::HashSet<SocketAddr> = std::collections::HashSet::new();

    // Source phase — wall-clock per verification task, active gauge.
    metrics.source_active.fetch_add(1, Ordering::Relaxed);
    let source_start = Instant::now();
    let mut source_completed = false;

    // 1) Immediately spawn the highest-quality leads: a cached announcing peer
    //    and/or the direct announce_peer. These are raced against the DHT
    //    lookup below so we never wait up to source_deadline behind querying
    //    mostly-dead DHT nodes for a peer that is live right now.
    if let Some(announcer) = announce_peer_cache.get(&info_hash) {
        if peers_seen.insert(announcer) {
            crate::trace_lifecycle!(
                &info_hash,
                "announce_peer_injected",
                stream = "fetch",
                peer = announcer.to_string()
            );
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
        let fetch_limit = fetch_limit.clone();
        metrics.fetch_attempts.add(1);
        set.spawn(async move {
            let permit_start = Instant::now();
            let _fetch_permit = fetch_limit
                .acquire_owned()
                .await
                .expect("fetch limit closed");
            let wait_us = permit_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
            saturating_add_atomic(&metrics.fetch_permit_wait_micros_total, wait_us);
            metrics
                .fetch_permit_acquisitions_total
                .fetch_add(1, Ordering::Relaxed);
            try_fetch(
                addr,
                ih,
                pid,
                metrics,
                cache,
                utp,
                po,
                "announce_peer",
                fetch_timeout,
                tcp_timeout,
                utp_timeout,
                limiter,
                transport_race_concurrent,
                connect_deadline,
            )
            .await
        });
    }

    // 2) Spawn the DHT lookup concurrently so it never blocks the direct race.
    let router_clone = router.clone();
    let metrics_clone = metrics.clone();
    let peer_cache_dht = peer_cache.clone();
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
            &peer_cache_dht,
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
                if !source_completed {
                    source_completed = true;
                    let us = source_start
                        .elapsed()
                        .as_micros()
                        .min(u64::MAX as u128) as u64;
                    saturating_add_atomic(&metrics.verify_source_micros_total, us);
                    metrics
                        .verify_source_completed_total
                        .fetch_add(1, Ordering::Relaxed);
                    let prev = metrics.source_active.fetch_sub(1, Ordering::Relaxed);
                    debug_assert!(prev > 0, "source_active underflow");
                }
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
                            let fetch_limit = fetch_limit.clone();
                            metrics.fetch_attempts.add(1);
                            set.spawn(async move {
                                let permit_start = Instant::now();
                                let _fetch_permit =
                                    fetch_limit.acquire_owned().await.expect("fetch limit closed");
                                let wait_us = permit_start
                                    .elapsed()
                                    .as_micros()
                                    .min(u64::MAX as u128) as u64;
                                saturating_add_atomic(
                                    &metrics.fetch_permit_wait_micros_total,
                                    wait_us,
                                );
                                metrics
                                    .fetch_permit_acquisitions_total
                                    .fetch_add(1, Ordering::Relaxed);
                                try_fetch(addr, ih, pid, metrics, cache, utp, po, "get_peers", fetch_timeout, tcp_timeout, utp_timeout, limiter, transport_race_concurrent, connect_deadline).await
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

    let drain_start = Instant::now();
    if !source_completed {
        let prev = metrics.source_active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(prev > 0, "source_active underflow on abort");
    }
    set.abort_all();
    source_fut.abort();
    let drain_us = drain_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.fetch_joinset_drain_micros_total, drain_us);
    metrics
        .fetch_joinset_drain_completed_total
        .fetch_add(1, Ordering::Relaxed);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    fn ok_after(ms: u64) -> impl std::future::Future<Output = Result<(), WireError>> {
        async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(())
        }
    }

    fn fail_after(
        ms: u64,
        err: WireError,
    ) -> impl std::future::Future<Output = Result<(), WireError>> {
        async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Err(err)
        }
    }

    fn immediate_ok() -> impl std::future::Future<Output = Result<(), WireError>> {
        std::future::ready(Ok(()))
    }

    fn immediate_fail(err: WireError) -> impl std::future::Future<Output = Result<(), WireError>> {
        std::future::ready(Err(err))
    }

    struct OutcomeRecord {
        transport: Transport,
        result: &'static str,
    }

    fn collector() -> (
        Arc<Mutex<Vec<OutcomeRecord>>>,
        impl FnMut(Transport, &'static str, u64),
    ) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let r = records.clone();
        let cb = move |transport: Transport, result: &'static str, _elapsed: u64| {
            r.lock().unwrap().push(OutcomeRecord { transport, result });
        };
        (records, cb)
    }

    #[tokio::test]
    async fn race_tcp_wins_cancels_utp() {
        let (records, cb) = collector();
        let res = race_transports(ok_after(10), ok_after(500), Duration::from_secs(5), cb)
            .await
            .unwrap();
        assert!(matches!(res.0, Transport::Tcp));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].result, "ok");
    }

    #[tokio::test]
    async fn race_utp_wins_when_tcp_fails() {
        let (records, cb) = collector();
        let res = race_transports(
            immediate_fail(WireError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "refused",
            ))),
            ok_after(20),
            Duration::from_secs(5),
            cb,
        )
        .await
        .unwrap();
        assert!(matches!(res.0, Transport::Utp));
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].transport, Transport::Tcp);
        assert_eq!(recs[0].result, "io_error");
        assert_eq!(recs[1].transport, Transport::Utp);
        assert_eq!(recs[1].result, "ok");
    }

    #[tokio::test]
    async fn race_both_fail_returns_tcp_error() {
        let (records, cb) = collector();
        let res = race_transports(
            immediate_fail(WireError::Timeout),
            fail_after(
                20,
                WireError::Io(std::io::Error::new(std::io::ErrorKind::Other, "err")),
            ),
            Duration::from_secs(5),
            cb,
        )
        .await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WireError::Timeout));
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].transport, Transport::Tcp);
        assert_eq!(recs[0].result, "timeout");
        assert_eq!(recs[1].transport, Transport::Utp);
    }

    #[tokio::test]
    async fn race_both_fail_utp_first() {
        let (records, cb) = collector();
        let res = race_transports(
            fail_after(40, WireError::Timeout),
            immediate_fail(WireError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "err",
            ))),
            Duration::from_secs(5),
            cb,
        )
        .await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WireError::Timeout));
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].transport, Transport::Utp);
        assert_eq!(recs[1].transport, Transport::Tcp);
    }

    #[tokio::test]
    async fn race_deadline_fires() {
        let (records, cb) = collector();
        let res = race_transports(
            ok_after(1000),
            ok_after(1000),
            Duration::from_millis(20),
            cb,
        )
        .await;
        assert!(matches!(res, Err(WireError::Timeout)));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 0);
    }

    #[tokio::test]
    async fn race_tcp_success_no_utp_outcome_recorded() {
        let (records, cb) = collector();
        let res = race_transports(immediate_ok(), ok_after(500), Duration::from_secs(5), cb)
            .await
            .unwrap();
        assert!(matches!(res.0, Transport::Tcp));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 1, "only the winner should be recorded");
        assert_eq!(recs[0].result, "ok");
    }

    #[tokio::test]
    async fn race_tcp_fast_fail_utp_slow_ok() {
        let (records, cb) = collector();
        let res = race_transports(
            immediate_fail(WireError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "refused",
            ))),
            ok_after(100),
            Duration::from_secs(5),
            cb,
        )
        .await
        .unwrap();
        assert!(matches!(res.0, Transport::Utp));
        let recs = records.lock().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].result, "io_error");
        assert_eq!(recs[1].result, "ok");
    }
}
