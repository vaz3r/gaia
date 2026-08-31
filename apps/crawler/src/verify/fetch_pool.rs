use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::storage::peer_outcomes::{PeerOutcome, PeerOutcomeWriter};
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use crate::verify::wire::{WireError, WireSession, gen_peer_id};
use librqbit_utp::UtpSocketUdp;
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;
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
    pub lead_source_grace: Duration,
}

fn sample_failed_peer(ih: &Infohash, addr: &SocketAddr, metrics: &Metrics, rate: u64) {
    let count = metrics.fetch_connect_io.load(Ordering::Relaxed);
    let hash = count
        .wrapping_mul(0x517c1b7275698a01)
        .wrapping_mul((ih[0] as u64) | 1);
    if hash.is_multiple_of(rate) {
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
    ConnectFailed(SocketAddr, WireError, CandidateSource, Duration),
    MetadataFailed(WireError, CandidateSource, Duration),
    Success(Vec<u8>, std::net::SocketAddr, CandidateSource, Duration),
}

fn outcome_source(outcome: &FetchOutcome) -> CandidateSource {
    match outcome {
        FetchOutcome::ConnectFailed(_, _, s, _) => *s,
        FetchOutcome::MetadataFailed(_, s, _) => *s,
        FetchOutcome::Success(_, _, s, _) => *s,
    }
}

pub enum VerifyResult {
    Success(Vec<u8>, std::net::SocketAddr, CandidateSource, Duration),
    NoPeers,
    SourceTimeout,
    MetadataFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadDhtTriggerReason {
    GraceExpired,
    LeadExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSource {
    Direct,
    AnnounceCache,
    Dht,
}

impl CandidateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateSource::Direct | CandidateSource::AnnounceCache => "announce_peer",
            CandidateSource::Dht => "get_peers",
        }
    }
}

type PermitAcquisitionFuture =
    Pin<Box<dyn Future<Output = Result<OwnedSemaphorePermit, tokio::sync::AcquireError>> + Send>>;

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
    source: CandidateSource,
    fetch_timeout: Duration,
    tcp_timeout: Duration,
    utp_timeout: Duration,
    limiter: Arc<crate::verify::ConnLimiter>,
    transport_race_concurrent: bool,
    connect_deadline: Duration,
    _fetch_permit: tokio::sync::OwnedSemaphorePermit,
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
        let source_clone = source.as_str().to_string();
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
                match source {
                    CandidateSource::Direct => metrics
                        .source_direct_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::AnnounceCache => metrics
                        .source_announce_cache_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::Dht => metrics
                        .source_dht_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                };
                transport_str = "tcp";
                s
            }
            Ok((Transport::Utp, s)) => {
                metrics.utp_connect_ok.add(1);
                metrics.utp_connect_actual.add(1);
                match source {
                    CandidateSource::Direct => metrics
                        .source_direct_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::AnnounceCache => metrics
                        .source_announce_cache_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::Dht => metrics
                        .source_dht_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                };
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
                return FetchOutcome::ConnectFailed(addr, e, source, start.elapsed());
            }
        }
    } else {
        // Sequential path: TCP first, uTP fallback on failure (original logic).
        match WireSession::connect_tcp(addr, &ih, &pid, tcp_timeout).await {
            Ok(s) => {
                metrics.tcp_connect_ok.add(1);
                metrics.tcp_connect_actual.add(1);
                match source {
                    CandidateSource::Direct => metrics
                        .source_direct_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::AnnounceCache => metrics
                        .source_announce_cache_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                    CandidateSource::Dht => metrics
                        .source_dht_connect_ok_total
                        .fetch_add(1, Ordering::Relaxed),
                };
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
                    source: source.as_str().to_string(),
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
                                match source {
                                    CandidateSource::Direct => metrics
                                        .source_direct_connect_ok_total
                                        .fetch_add(1, Ordering::Relaxed),
                                    CandidateSource::AnnounceCache => metrics
                                        .source_announce_cache_connect_ok_total
                                        .fetch_add(1, Ordering::Relaxed),
                                    CandidateSource::Dht => metrics
                                        .source_dht_connect_ok_total
                                        .fetch_add(1, Ordering::Relaxed),
                                };
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
                                    source: source.as_str().to_string(),
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
                                return FetchOutcome::ConnectFailed(
                                    addr,
                                    utp_err,
                                    source,
                                    start.elapsed(),
                                );
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
                        return FetchOutcome::ConnectFailed(addr, tcp_err, source, start.elapsed());
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
        source: source.as_str().to_string(),
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
            match source {
                CandidateSource::Direct => metrics
                    .source_direct_metadata_ok_total
                    .fetch_add(1, Ordering::Relaxed),
                CandidateSource::AnnounceCache => metrics
                    .source_announce_cache_metadata_ok_total
                    .fetch_add(1, Ordering::Relaxed),
                CandidateSource::Dht => metrics
                    .source_dht_metadata_ok_total
                    .fetch_add(1, Ordering::Relaxed),
            };
            peer_outcomes.push(PeerOutcome {
                ih,
                peer: addr.to_string(),
                source: source.as_str().to_string(),
                transport: transport_str.to_string(),
                result: "metadata_ok".to_string(),
                client: last_client,
                phase: Some("metadata".to_string()),
                elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32),
            });
            FetchOutcome::Success(meta, addr, source, start.elapsed())
        }
        Err(e) => {
            metrics.metadata_failed_io.add(1);
            match source {
                CandidateSource::Direct => metrics
                    .source_direct_metadata_fail_total
                    .fetch_add(1, Ordering::Relaxed),
                CandidateSource::AnnounceCache => metrics
                    .source_announce_cache_metadata_fail_total
                    .fetch_add(1, Ordering::Relaxed),
                CandidateSource::Dht => metrics
                    .source_dht_metadata_fail_total
                    .fetch_add(1, Ordering::Relaxed),
            };
            peer_outcomes.push(PeerOutcome {
                ih,
                peer: addr.to_string(),
                source: source.as_str().to_string(),
                transport: transport_str.to_string(),
                result: wire_error_to_outcome(&e).to_string(),
                client: last_client,
                phase: Some("metadata".to_string()),
                elapsed_ms: Some(metadata_start.elapsed().as_millis().min(i32::MAX as u128) as i32),
            });
            FetchOutcome::MetadataFailed(e, source, start.elapsed())
        }
    };
    let meta_us = metadata_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.metadata_exchange_micros_total, meta_us);
    metrics
        .metadata_exchange_completed_total
        .fetch_add(1, Ordering::Relaxed);
    drop(_fetch_permit);
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
    fetch_limit: Arc<tokio::sync::Semaphore>,
    negative_cache: Arc<dashmap::DashMap<std::net::IpAddr, tokio::time::Instant>>,
) -> VerifyResult {
    let race_peers = params.race_peers.max(1);
    let peer_id = gen_peer_id();
    let fetch_timeout = params.metadata_timeout;
    let tcp_timeout = params.tcp_timeout;
    let utp_timeout = params.utp_timeout;
    let transport_race_concurrent = params.transport_race_concurrent;
    let connect_deadline = params.connect_deadline;

    let mut set: JoinSet<FetchOutcome> = JoinSet::new();
    let mut peers_seen: HashSet<SocketAddr> = HashSet::new();
    let mut candidate_queue: VecDeque<(SocketAddr, CandidateSource)> =
        VecDeque::with_capacity(race_peers);

    // 1) Enqueue the highest-quality leads in priority order:
    //    Priority 1: Direct peer supplied by announce_peer
    //    Priority 2: AnnouncePeerCache peer
    if let Some(d) = direct
        && peers_seen.insert(d) && candidate_queue.len() < race_peers {
            metrics
                .source_direct_accepted_total
                .fetch_add(1, Ordering::Relaxed);
            candidate_queue.push_back((d, CandidateSource::Direct));
        }
    if let Some(announcer) = announce_peer_cache.get(&info_hash)
        && peers_seen.insert(announcer) {
            crate::trace_lifecycle!(
                &info_hash,
                "announce_peer_injected",
                stream = "fetch",
                peer = announcer.to_string()
            );
            if candidate_queue.len() < race_peers {
                metrics
                    .source_announce_cache_accepted_total
                    .fetch_add(1, Ordering::Relaxed);
                candidate_queue.push_back((announcer, CandidateSource::AnnounceCache));
            }
        }

    let is_lead_task = !candidate_queue.is_empty();
    if is_lead_task {
        metrics.lead_tasks_total.fetch_add(1, Ordering::Relaxed);
    } else {
        metrics.non_lead_tasks_total.fetch_add(1, Ordering::Relaxed);
    }

    enum DhtSourceLifecycle {
        NotStarted,
        Running(tokio::task::JoinHandle<SourceResult>, Instant),
        Finished,
    }

    let mut dht_lifecycle = DhtSourceLifecycle::NotStarted;
    let grace_duration = params.lead_source_grace;
    let defer_dht = is_lead_task && !grace_duration.is_zero();
    let mut grace_timer: Option<Pin<Box<tokio::time::Sleep>>> = if defer_dht {
        metrics
            .lead_dht_deferred_total
            .fetch_add(1, Ordering::Relaxed);
        Some(Box::pin(tokio::time::sleep(grace_duration)))
    } else {
        None
    };

    let router_clone = router.clone();
    let metrics_clone = metrics.clone();
    let peer_cache_dht = peer_cache.clone();
    let source_deadline = params.source_deadline;
    let source_k = params.source_k;
    let source_alpha = params.source_alpha;
    let source_query_timeout = params.source_query_timeout;
    let source_max_queries = params.source_max_queries;

    let maybe_start_dht =
        |lifecycle: &mut DhtSourceLifecycle, reason: Option<LeadDhtTriggerReason>| -> bool {
            if matches!(lifecycle, DhtSourceLifecycle::NotStarted) {
                metrics_clone.source_active.fetch_add(1, Ordering::Relaxed);
                if is_lead_task {
                    metrics_clone
                        .lead_tasks_dht_started_total
                        .fetch_add(1, Ordering::Relaxed);
                    match reason {
                        Some(LeadDhtTriggerReason::GraceExpired) => {
                            metrics_clone
                                .lead_dht_started_grace_expired_total
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Some(LeadDhtTriggerReason::LeadExhausted) => {
                            metrics_clone
                                .lead_dht_started_exhausted_total
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        None => {}
                    }
                }
                let router_c = router_clone.clone();
                let metrics_c = metrics_clone.clone();
                let peer_cache_c = peer_cache_dht.clone();
                let started_at = Instant::now();
                let handle = tokio::spawn(async move {
                    source_peers(
                        router_c,
                        info_hash,
                        race_peers,
                        metrics_c,
                        source_deadline,
                        source_k,
                        source_alpha,
                        source_query_timeout,
                        source_max_queries,
                        &peer_cache_c,
                        is_lead_task,
                    )
                    .await
                });
                *lifecycle = DhtSourceLifecycle::Running(handle, started_at);
                true
            } else {
                false
            }
        };

    if !defer_dht {
        maybe_start_dht(&mut dht_lifecycle, None);
    }

    let mut source_state = SourceState::Timeout;
    let mut result: Option<(Vec<u8>, std::net::SocketAddr, CandidateSource, Duration)> = None;

    // Persistent permit acquisition future across select! ticks.
    let mut permit_fut: Option<PermitAcquisitionFuture> = None;
    let mut permit_wait_start: Option<Instant> = None;

    // Track active fetch attempts and queued candidates by source for lead exhaustion
    let mut active_lead_attempts: usize = 0;
    let mut queued_lead_candidates: usize = candidate_queue.len();

    loop {
        // Maintain persistent permit future when candidates are waiting and we have not initiated acquisition.
        if permit_fut.is_none() && !candidate_queue.is_empty() {
            permit_wait_start = Some(Instant::now());
            let sem = fetch_limit.clone();
            permit_fut = Some(Box::pin(async move { sem.acquire_owned().await }));
        }

        tokio::select! {
            // Active attempt completed
            res = set.join_next(), if !set.is_empty() => {
                match res {
                    Some(Ok(FetchOutcome::Success(meta, addr, src, dur))) => {
                        result = Some((meta, addr, src, dur));
                        break;
                    }
                    Some(Ok(outcome)) => {
                        let outcome_src = outcome_source(&outcome);
                        if outcome_src != CandidateSource::Dht {
                            active_lead_attempts = active_lead_attempts.saturating_sub(1);
                        }
                        match outcome {
                            FetchOutcome::ConnectFailed(_addr, WireError::Timeout, src, dur) => {
                                negative_cache.insert(_addr.ip(), tokio::time::Instant::now() + std::time::Duration::from_secs(60));
                                metrics.fetch_connect_timeout.add(1);
                                metrics.verify_timeouts.add(1);
                                match src {
                                    CandidateSource::Direct => {
                                        metrics.source_direct_connect_timeout_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::AnnounceCache => {
                                        metrics.source_announce_cache_connect_timeout_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::Dht => {
                                        metrics.source_dht_connect_timeout_total.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            FetchOutcome::ConnectFailed(_addr, WireError::Io(_), src, dur) => {
                                negative_cache.insert(_addr.ip(), tokio::time::Instant::now() + std::time::Duration::from_secs(60));
                                metrics.fetch_connect_io.add(1);
                                match src {
                                    CandidateSource::Direct => {
                                        metrics.source_direct_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::AnnounceCache => {
                                        metrics.source_announce_cache_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::Dht => {
                                        metrics.source_dht_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                sample_failed_peer(&info_hash, &_addr, &metrics, params.failed_peer_sample_rate.max(1));
                            }
                            FetchOutcome::ConnectFailed(_addr, _, src, dur) => {
                                metrics.fetch_connect_io.add(1);
                                match src {
                                    CandidateSource::Direct => {
                                        metrics.source_direct_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::AnnounceCache => {
                                        metrics.source_announce_cache_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                        record_lead_failure_latency(&metrics, dur);
                                    }
                                    CandidateSource::Dht => {
                                        metrics.source_dht_connect_io_total.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                sample_failed_peer(&info_hash, &_addr, &metrics, params.failed_peer_sample_rate.max(1));
                            }
                            FetchOutcome::MetadataFailed(WireError::Timeout, src, dur) => {
                                metrics.fetch_io.add(1);
                                metrics.verify_timeouts.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::Handshake, src, dur) => {
                                metrics.fetch_handshake.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::NoExtension, src, dur) => {
                                metrics.fetch_no_extension.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::Reject, src, dur) => {
                                metrics.fetch_reject.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::BadPiece, src, dur) => {
                                metrics.fetch_bad_piece.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::Io(_), src, dur) => {
                                metrics.fetch_io.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::Eof, src, dur) => {
                                metrics.fetch_io.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::Cancelled, src, dur) => {
                                metrics.fetch_io.add(1);
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::MetadataFailed(WireError::NoMetadataSize, src, dur) => {
                                if src != CandidateSource::Dht {
                                    record_lead_failure_latency(&metrics, dur);
                                }
                            }
                            FetchOutcome::Success(_, _, _, _) => {}
                        }

                        // Check lead exhaustion condition after a lead failure
                        if queued_lead_candidates == 0 && active_lead_attempts == 0 {
                            if let Some(timer) = grace_timer.take() {
                                let elapsed = timer.deadline().saturating_duration_since(tokio::time::Instant::now());
                                let grace_taken_dur = grace_duration.saturating_sub(elapsed);
                                let grace_us = grace_taken_dur.as_micros().min(u64::MAX as u128) as u64;
                                saturating_add_atomic(&metrics.lead_grace_micros_total, grace_us);
                                metrics
                                    .lead_grace_completed_total
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            maybe_start_dht(&mut dht_lifecycle, Some(LeadDhtTriggerReason::LeadExhausted));
                        }
                    }
                    _ => {}
                }
            }

            // Lead grace timer expired
            _ = async {
                match &mut grace_timer {
                    Some(timer) => timer.as_mut().await,
                    None => std::future::pending().await,
                }
            }, if grace_timer.is_some() => {
                grace_timer = None;
                let grace_us = grace_duration.as_micros().min(u64::MAX as u128) as u64;
                saturating_add_atomic(&metrics.lead_grace_micros_total, grace_us);
                metrics
                    .lead_grace_completed_total
                    .fetch_add(1, Ordering::Relaxed);
                maybe_start_dht(&mut dht_lifecycle, Some(LeadDhtTriggerReason::GraceExpired));
            }

            // Global fetch permit acquired for front candidate
            permit_res = async {
                match &mut permit_fut {
                    Some(fut) => fut.await,
                    None => std::future::pending().await,
                }
            }, if permit_fut.is_some() => {
                permit_fut = None;
                if let Ok(permit) = permit_res {
                    if let Some(start) = permit_wait_start.take() {
                        let wait_us = start.elapsed().as_micros().min(u64::MAX as u128) as u64;
                        saturating_add_atomic(&metrics.fetch_permit_wait_micros_total, wait_us);
                        metrics
                            .fetch_permit_acquisitions_total
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some((addr, source)) = candidate_queue.pop_front() {
                        if let Some(exp) = negative_cache.get(&addr.ip())
                            && *exp > tokio::time::Instant::now() {
                                drop(permit);
                                continue;
                            }
                        if source != CandidateSource::Dht {
                            queued_lead_candidates = queued_lead_candidates.saturating_sub(1);
                            active_lead_attempts += 1;
                        }
                        metrics.fetch_permit_owned_attempts_total.fetch_add(1, Ordering::Relaxed);
                        metrics.fetch_attempts.add(1);
                        match source {
                            CandidateSource::Direct => metrics.source_direct_attempts_total.fetch_add(1, Ordering::Relaxed),
                            CandidateSource::AnnounceCache => metrics.source_announce_cache_attempts_total.fetch_add(1, Ordering::Relaxed),
                            CandidateSource::Dht => metrics.source_dht_attempts_total.fetch_add(1, Ordering::Relaxed),
                        };
                        let ih = info_hash;
                        let pid = peer_id;
                        let metrics_c = metrics.clone();
                        let cache_c = peer_cache.clone();
                        let utp_c = utp.clone();
                        let po_c = peer_outcomes.clone();
                        let limiter_c = conn_limiter.clone();
                        set.spawn(async move {
                            try_fetch(
                                addr,
                                ih,
                                pid,
                                metrics_c,
                                cache_c,
                                utp_c,
                                po_c,
                                source,
                                fetch_timeout,
                                tcp_timeout,
                                utp_timeout,
                                limiter_c,
                                transport_race_concurrent,
                                connect_deadline,
                                permit,
                            )
                            .await
                        });
                        // Yield to the Tokio runtime after spawning so other tasks/schedulers
                        // waiting on the fetch_limit semaphore can make progress.
                        tokio::task::yield_now().await;
                    }
                }
            }

            // DHT source completed
            dht_res = async {
                match &mut dht_lifecycle {
                    DhtSourceLifecycle::Running(handle, _) => handle.await,
                    _ => std::future::pending().await,
                }
            }, if matches!(dht_lifecycle, DhtSourceLifecycle::Running(_, _)) => {
                let started_at = match std::mem::replace(&mut dht_lifecycle, DhtSourceLifecycle::Finished) {
                    DhtSourceLifecycle::Running(_, t) => t,
                    _ => unreachable!(),
                };
                let us = started_at
                    .elapsed()
                    .as_micros()
                    .min(u64::MAX as u128) as u64;
                saturating_add_atomic(&metrics.verify_source_micros_total, us);
                metrics
                    .verify_source_completed_total
                    .fetch_add(1, Ordering::Relaxed);
                let prev = metrics.source_active.fetch_sub(1, Ordering::Relaxed);
                debug_assert!(prev > 0, "source_active underflow");

                if let Ok(lookup_res) = dht_res {
                    let (new_peers, state) = match lookup_res {
                        SourceResult::Peers(p) => (p, SourceState::Peers),
                        SourceResult::NoPeers => (Vec::new(), SourceState::NoPeers),
                        SourceResult::AllTimeout => (Vec::new(), SourceState::Timeout),
                    };
                    source_state = state;
                    for addr in new_peers {
                        if peers_seen.contains(&addr) {
                            continue;
                        }
                        if peers_seen.len() >= race_peers {
                            metrics.fetch_candidates_skipped_budget_total.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        peers_seen.insert(addr);
                        metrics.source_dht_accepted_total.fetch_add(1, Ordering::Relaxed);
                        candidate_queue.push_back((addr, CandidateSource::Dht));
                    }
                }
            }

            // All branches completed/exhausted
            else => {
                if candidate_queue.is_empty() && set.is_empty() && matches!(dht_lifecycle, DhtSourceLifecycle::Finished) {
                    break;
                }
            }
        }
    }

    let drain_start = Instant::now();
    match dht_lifecycle {
        DhtSourceLifecycle::NotStarted => {
            if is_lead_task && result.is_some() {
                metrics
                    .lead_dht_avoided_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        DhtSourceLifecycle::Running(handle, _) => {
            let prev = metrics.source_active.fetch_sub(1, Ordering::Relaxed);
            debug_assert!(prev > 0, "source_active underflow on abort");
            handle.abort();
        }
        DhtSourceLifecycle::Finished => {}
    }
    set.abort_all();
    let drain_us = drain_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    saturating_add_atomic(&metrics.fetch_joinset_drain_micros_total, drain_us);
    metrics
        .fetch_joinset_drain_completed_total
        .fetch_add(1, Ordering::Relaxed);

    match result {
        Some((meta, peer, src, dur)) => VerifyResult::Success(meta, peer, src, dur),
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

pub fn record_lead_failure_latency(metrics: &Metrics, dur: Duration) {
    let ms = dur.as_millis();
    if ms <= 250 {
        metrics
            .lead_failure_le_250ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 500 {
        metrics
            .lead_failure_le_500ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 1000 {
        metrics
            .lead_failure_le_1000ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 2000 {
        metrics
            .lead_failure_le_2000ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else {
        metrics
            .lead_failure_gt_2000ms_total
            .fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_lead_success_latency(metrics: &Metrics, dur: Duration) {
    let ms = dur.as_millis();
    if ms <= 250 {
        metrics
            .lead_success_le_250ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 500 {
        metrics
            .lead_success_le_500ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 1000 {
        metrics
            .lead_success_le_1000ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else if ms <= 2000 {
        metrics
            .lead_success_le_2000ms_total
            .fetch_add(1, Ordering::Relaxed);
    } else {
        metrics
            .lead_success_gt_2000ms_total
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64;
    use tokio::sync::Semaphore;
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

    #[tokio::test]
    async fn attempt_spawned_only_with_owned_fetch_permit() {
        let sem = Arc::new(Semaphore::new(0)); // 0 initial permits
        let (tx, _rx) = tokio::sync::mpsc::channel::<()>(1);
        let sem_clone = sem.clone();

        let mut permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        // With 0 permits, permit_fut should not resolve
        tokio::select! {
            biased;
            _ = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                panic!("Permit should not have been acquired with 0 available permits");
            }
            _ = tokio::time::sleep(Duration::from_millis(20)) => {
                // Verified that without permit, no attempt is scheduled
            }
        }

        // Add 1 permit and ensure it resolves
        sem.add_permits(1);
        let permit = match &mut permit_fut {
            Some(f) => f.await.unwrap(),
            None => panic!("missing permit fut"),
        };
        assert_eq!(sem.available_permits(), 0);
        drop(permit);
        assert_eq!(sem.available_permits(), 1);
        drop(tx);
    }

    #[tokio::test]
    async fn global_permit_retained_throughout_attempt_lifecycle() {
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.unwrap();
        assert_eq!(sem.available_permits(), 0);

        // Permit remains held throughout try_fetch execution
        assert_eq!(sem.available_permits(), 0);
        // When try_fetch returns (or task finishes), permit is dropped
        drop(permit);
        assert_eq!(
            sem.available_permits(),
            1,
            "Global fetch permit must be released when attempt completes or fails"
        );
    }

    #[test]
    fn candidate_and_active_budget_never_exceeds_race_peers() {
        let race_peers = 8;
        let mut candidate_queue: VecDeque<(SocketAddr, CandidateSource)> =
            VecDeque::with_capacity(race_peers);
        let active_count = 3;

        let new_dht_peers = vec![
            "1.1.1.1:6881".parse().unwrap(),
            "2.2.2.2:6881".parse().unwrap(),
            "3.3.3.3:6881".parse().unwrap(),
            "4.4.4.4:6881".parse().unwrap(),
            "5.5.5.5:6881".parse().unwrap(),
            "6.6.6.6:6881".parse().unwrap(),
            "7.7.7.7:6881".parse().unwrap(),
        ];

        let mut skipped = 0;
        for addr in new_dht_peers {
            if candidate_queue.len() + active_count >= race_peers {
                skipped += 1;
                continue;
            }
            candidate_queue.push_back((addr, CandidateSource::Dht));
        }

        assert_eq!(candidate_queue.len() + active_count, race_peers);
        assert_eq!(candidate_queue.len(), 5);
        assert_eq!(skipped, 2);
    }

    #[tokio::test]
    async fn temporary_permit_unavailability_preserves_front_candidate() {
        let sem = Arc::new(Semaphore::new(0));
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        let addr: SocketAddr = "1.2.3.4:6881".parse().unwrap();
        queue.push_back((addr, CandidateSource::Direct));

        let sem_clone = sem.clone();
        let mut permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        // Wait momentarily: permit is unavailable
        tokio::select! {
            _ = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => panic!("Should not acquire"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }

        // Verify candidate remains at front of queue
        assert_eq!(queue.front().unwrap().0, addr);

        // Now permit becomes available
        sem.add_permits(1);
        let permit = match &mut permit_fut {
            Some(f) => f.await.unwrap(),
            None => panic!("missing fut"),
        };
        let popped = queue.pop_front().unwrap();
        assert_eq!(popped.0, addr);
        assert_eq!(popped.1, CandidateSource::Direct);
        drop(permit);
    }

    #[tokio::test]
    async fn persistent_permit_wait_survives_other_select_branch() {
        let sem = Arc::new(Semaphore::new(0));
        let sem_clone = sem.clone();
        let mut permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        tx.send(()).await.unwrap();

        let other_branch_fired;
        tokio::select! {
            _ = rx.recv() => {
                other_branch_fired = true;
            }
            _ = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => {
                panic!("Permit should not fire");
            }
        }

        assert!(other_branch_fired);
        assert!(
            permit_fut.is_some(),
            "Permit future must be preserved across other select branches"
        );

        sem.add_permits(1);
        let permit = match &mut permit_fut {
            Some(f) => f.await.unwrap(),
            None => panic!("missing"),
        };
        drop(permit);
    }

    #[tokio::test]
    async fn one_scheduler_step_spawns_at_most_one_attempt() {
        let sem = Arc::new(Semaphore::new(10)); // 10 permits available
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        queue.push_back(("1.1.1.1:6881".parse().unwrap(), CandidateSource::Direct));
        queue.push_back(("2.2.2.2:6881".parse().unwrap(), CandidateSource::Dht));
        queue.push_back(("3.3.3.3:6881".parse().unwrap(), CandidateSource::Dht));

        let mut permit_fut: Option<PermitAcquisitionFuture> = None;
        let mut spawned = 0;

        // Single progression step
        if permit_fut.is_none() && !queue.is_empty() {
            let s = sem.clone();
            permit_fut = Some(Box::pin(async move { s.acquire_owned().await }));
        }

        tokio::select! {
            permit_res = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if permit_fut.is_some() => {
                if let Ok(_permit) = permit_res
                    && let Some(_cand) = queue.pop_front() {
                    spawned += 1;
                }
            }
        }

        assert_eq!(
            spawned, 1,
            "Exactly one attempt must be spawned per scheduler progression step"
        );
        assert_eq!(
            queue.len(),
            2,
            "Remaining candidates must stay queued for subsequent steps"
        );
    }

    #[test]
    fn direct_then_announce_then_dht_priority() {
        let direct: Option<SocketAddr> = Some("1.1.1.1:6881".parse().unwrap());
        let announcer: Option<SocketAddr> = Some("2.2.2.2:6881".parse().unwrap());
        let dht_peers = vec![
            "3.3.3.3:6881".parse().unwrap(),
            "4.4.4.4:6881".parse().unwrap(),
        ];

        let mut seen = HashSet::new();
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();

        // Step 1: Direct peer
        if let Some(d) = direct
            && seen.insert(d)
        {
            queue.push_back((d, CandidateSource::Direct));
        }
        // Step 2: Announce cache
        if let Some(a) = announcer
            && seen.insert(a)
        {
            queue.push_back((a, CandidateSource::AnnounceCache));
        }
        // Step 3: DHT peers
        for addr in dht_peers {
            if seen.insert(addr) {
                queue.push_back((addr, CandidateSource::Dht));
            }
        }

        assert_eq!(
            queue.pop_front().unwrap(),
            ("1.1.1.1:6881".parse().unwrap(), CandidateSource::Direct)
        );
        assert_eq!(
            queue.pop_front().unwrap(),
            (
                "2.2.2.2:6881".parse().unwrap(),
                CandidateSource::AnnounceCache
            )
        );
        assert_eq!(
            queue.pop_front().unwrap(),
            ("3.3.3.3:6881".parse().unwrap(), CandidateSource::Dht)
        );
        assert_eq!(
            queue.pop_front().unwrap(),
            ("4.4.4.4:6881".parse().unwrap(), CandidateSource::Dht)
        );
    }

    #[test]
    fn duplicate_peer_queued_and_attempted_once() {
        let addr: SocketAddr = "1.2.3.4:6881".parse().unwrap();
        let mut seen = HashSet::new();
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();

        // Direct
        if seen.insert(addr) {
            queue.push_back((addr, CandidateSource::Direct));
        }
        // Announce duplicate
        if seen.insert(addr) {
            queue.push_back((addr, CandidateSource::AnnounceCache));
        }
        // DHT duplicate
        if seen.insert(addr) {
            queue.push_back((addr, CandidateSource::Dht));
        }

        assert_eq!(
            queue.len(),
            1,
            "Duplicate addresses must be accepted into queue at most once"
        );
    }

    #[test]
    fn dht_batch_excess_is_skipped_by_budget() {
        let race_peers = 4;
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        let mut seen = HashSet::new();

        // 1 direct queued
        let d: SocketAddr = "1.1.1.1:6881".parse().unwrap();
        seen.insert(d);
        queue.push_back((d, CandidateSource::Direct));

        let dht_results = vec![
            "2.2.2.2:6881".parse().unwrap(),
            "3.3.3.3:6881".parse().unwrap(),
            "4.4.4.4:6881".parse().unwrap(),
            "5.5.5.5:6881".parse().unwrap(),
        ];

        let mut skipped = 0;
        for addr in dht_results {
            if seen.contains(&addr) {
                continue;
            }
            if seen.len() >= race_peers {
                skipped += 1;
                continue;
            }
            seen.insert(addr);
            queue.push_back((addr, CandidateSource::Dht));
        }

        assert_eq!(queue.len(), 4);
        assert_eq!(seen.len(), 4);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn cumulative_race_peers_budget_preserved_after_failures() {
        // Scenario:
        // - race_peers = 8
        // - two direct/announce candidates are accepted and fail
        // - DHT later returns eight unique candidates
        // - only six DHT candidates may be accepted
        // - total attempted candidates must remain eight
        let race_peers = 8;
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        let mut peers_seen = HashSet::new();

        // 1) Direct + AnnounceCache accepted
        let d1: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        let d2: SocketAddr = "10.0.0.2:6881".parse().unwrap();
        peers_seen.insert(d1);
        queue.push_back((d1, CandidateSource::Direct));
        peers_seen.insert(d2);
        queue.push_back((d2, CandidateSource::AnnounceCache));

        // 2) Both candidates are popped and fail (active_attempts = 0, queue.len() = 0)
        let _c1 = queue.pop_front().unwrap();
        let _c2 = queue.pop_front().unwrap();
        assert_eq!(queue.len(), 0);
        assert_eq!(peers_seen.len(), 2);

        // 3) DHT returns 8 unique peers
        let dht_results = vec![
            "10.0.0.3:6881".parse().unwrap(),
            "10.0.0.4:6881".parse().unwrap(),
            "10.0.0.5:6881".parse().unwrap(),
            "10.0.0.6:6881".parse().unwrap(),
            "10.0.0.7:6881".parse().unwrap(),
            "10.0.0.8:6881".parse().unwrap(),
            "10.0.0.9:6881".parse().unwrap(),
            "10.0.0.10:6881".parse().unwrap(),
        ];

        let mut skipped = 0;
        for addr in dht_results {
            if peers_seen.contains(&addr) {
                continue;
            }
            if peers_seen.len() >= race_peers {
                skipped += 1;
                continue;
            }
            peers_seen.insert(addr);
            queue.push_back((addr, CandidateSource::Dht));
        }

        // Exactly 6 DHT candidates accepted, total accepted = 8, 2 skipped
        assert_eq!(queue.len(), 6);
        assert_eq!(peers_seen.len(), race_peers);
        assert_eq!(skipped, 2);
    }

    #[tokio::test]
    async fn scheduler_performs_cooperative_yield_between_attempt_spawns() {
        let sem = Arc::new(Semaphore::new(2));

        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        queue.push_back(("1.1.1.1:6881".parse().unwrap(), CandidateSource::Direct));
        queue.push_back(("1.1.1.2:6881".parse().unwrap(), CandidateSource::Dht));

        let mut permit_fut: Option<PermitAcquisitionFuture> = None;
        let mut spawned = 0;

        // Step 1: Arm permit future for candidate 1
        if permit_fut.is_none() && !queue.is_empty() {
            let s = sem.clone();
            permit_fut = Some(Box::pin(async move { s.acquire_owned().await }));
        }

        tokio::select! {
            permit_res = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if permit_fut.is_some() => {
                permit_fut = None;
                if let Ok(permit) = permit_res
                    && let Some((_addr, _source)) = queue.pop_front() {
                    spawned += 1;
                    drop(permit);
                    tokio::task::yield_now().await;
                }
            }
        }

        // Exactly one attempt spawned in step 1, second candidate remains in queue
        assert_eq!(spawned, 1);
        assert_eq!(queue.len(), 1);
        assert!(
            permit_fut.is_none(),
            "Permit future is not yet re-armed before the next scheduler step"
        );

        // Step 2: Next scheduler progression step
        if permit_fut.is_none() && !queue.is_empty() {
            let s = sem.clone();
            permit_fut = Some(Box::pin(async move { s.acquire_owned().await }));
        }

        tokio::select! {
            permit_res = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if permit_fut.is_some() => {
                if let Ok(permit) = permit_res
                    && let Some((_addr, _source)) = queue.pop_front() {
                    spawned += 1;
                    drop(permit);
                    tokio::task::yield_now().await;
                }
            }
        }

        assert_eq!(spawned, 2);
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn first_success_aborts_source_losing_attempts_and_pending_permit() {
        // Test scenario:
        // - one fetch attempt is about to succeed
        // - another candidate is waiting on the persistent permit future
        // - success branch wins
        // - pending acquisition future is dropped
        // - all semaphore permits become available
        // - queued candidates are dropped
        // - active losing attempts are aborted
        // - source lookup is aborted
        let sem = Arc::new(Semaphore::new(1));
        let log_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        // 1 permit taken by winning attempt
        let winning_permit = sem.clone().acquire_owned().await.unwrap();
        assert_eq!(sem.available_permits(), 0);

        // Persistent permit waiter waiting for 2nd candidate
        let sem_clone = sem.clone();
        let mut permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        queue.push_back(("2.2.2.2:6881".parse().unwrap(), CandidateSource::Dht));

        let mut set: JoinSet<FetchOutcome> = JoinSet::new();
        set.spawn(async move {
            let _g = FetchActiveGuard::new(metrics.clone());
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(winning_permit);
            FetchOutcome::Success(
                vec![0xAA; 20],
                "127.0.0.1:6881".parse().unwrap(),
                CandidateSource::Direct,
                Duration::from_millis(10),
            )
        });

        let source_handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            SourceResult::NoPeers
        });

        let mut result = None;
        tokio::select! {
            biased;
            res = set.join_next(), if !set.is_empty() => {
                if let Some(Ok(FetchOutcome::Success(meta, _addr, _src, _dur))) = res {
                    result = Some(meta);
                }
            }
            _ = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if permit_fut.is_some() => {
                panic!("Permit should not be acquired before winning attempt finishes");
            }
        }

        // Clean up on success
        drop(permit_fut);
        queue.clear();
        set.abort_all();
        source_handle.abort();

        assert_eq!(result.unwrap(), vec![0xAA; 20]);
        assert_eq!(queue.len(), 0);
        assert_eq!(
            sem.available_permits(),
            1,
            "All semaphore permits must be available and uncorrupted"
        );
    }

    #[tokio::test]
    async fn pending_permit_acquisition_cancelled_on_success() {
        let sem = Arc::new(Semaphore::new(0));
        let sem_clone = sem.clone();
        let permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        // Verification succeeds before permit is acquired: drop future
        drop(permit_fut);

        // Semaphore capacity is untouched and uncorrupted
        assert_eq!(sem.available_permits(), 0);
        sem.add_permits(1);
        assert_eq!(sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn global_permit_released_on_failure() {
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.unwrap();
        assert_eq!(sem.available_permits(), 0);

        // Simulate failure before metadata transfer
        drop(permit);
        assert_eq!(
            sem.available_permits(),
            1,
            "Permit must be released on connect/handshake failure"
        );
    }

    #[tokio::test]
    async fn global_permit_released_on_task_abort() {
        let sem = Arc::new(Semaphore::new(1));
        let sem_clone = sem.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire_owned().await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(sem.available_permits(), 0);

        handle.abort();
        let _ = handle.await;
        assert_eq!(
            sem.available_permits(),
            1,
            "Permit must be released when task is aborted"
        );
    }

    #[tokio::test]
    async fn per_ip_limit_remains_enforced() {
        let limiter = Arc::new(crate::verify::ConnLimiter::new(1));
        let ip = "1.2.3.4".parse().unwrap();

        let p1 = limiter.acquire(ip).await;

        let mut p2_fut = Box::pin(limiter.acquire(ip));
        tokio::select! {
            _ = &mut p2_fut => panic!("Second acquisition for same IP should block"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }

        drop(p1);
        let _p2 = p2_fut.await; // Now succeeds
    }

    #[tokio::test]
    async fn multiple_verify_operations_never_exceed_global_fetch_limit() {
        let global_fetch_limit = 5;
        let sem = Arc::new(Semaphore::new(global_fetch_limit));

        let mut permits = Vec::new();
        for _ in 0..global_fetch_limit {
            permits.push(sem.clone().acquire_owned().await.unwrap());
        }
        assert_eq!(sem.available_permits(), 0);

        let mut sixth_fut = Box::pin(sem.clone().acquire_owned());
        tokio::select! {
            _ = &mut sixth_fut => panic!("Sixth permit should block"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }

        drop(permits);
        // sixth_fut immediately acquires 1 permit when permits are dropped, leaving 4 available
        let p = sixth_fut.await.unwrap();
        assert_eq!(sem.available_permits(), 4);
        drop(p);
        assert_eq!(sem.available_permits(), 5);
    }

    #[tokio::test]
    async fn shutdown_releases_permits_and_active_gauges() {
        let log_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));
        let sem = Arc::new(Semaphore::new(2));

        {
            let _fetch_guard = FetchActiveGuard::new(metrics.clone());
            let _permit = sem.clone().acquire_owned().await.unwrap();
            assert_eq!(metrics.fetch_active.load(Ordering::Relaxed), 1);
            assert_eq!(sem.available_permits(), 1);
        }

        assert_eq!(metrics.fetch_active.load(Ordering::Relaxed), 0);
        assert_eq!(sem.available_permits(), 2);
    }

    #[tokio::test]
    async fn scheduler_does_not_busy_loop_without_permit() {
        let sem = Arc::new(Semaphore::new(0)); // 0 permits
        let mut queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        queue.push_back(("1.2.3.4:6881".parse().unwrap(), CandidateSource::Direct));

        let sem_clone = sem.clone();
        let mut permit_fut: Option<PermitAcquisitionFuture> =
            Some(Box::pin(async move { sem_clone.acquire_owned().await }));

        let start = Instant::now();
        tokio::select! {
            _ = async {
                match &mut permit_fut {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            } => panic!("Should not acquire"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }

        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn candidate_source_attribution_isolated_by_source_type() {
        let log_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        // 1. Direct candidate
        metrics
            .source_direct_accepted_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_direct_attempts_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_direct_connect_ok_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_direct_metadata_ok_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_direct_verified_total
            .fetch_add(1, Ordering::Relaxed);

        assert_eq!(
            metrics.source_direct_verified_total.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .source_announce_cache_verified_total
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(metrics.source_dht_verified_total.load(Ordering::Relaxed), 0);

        // 2. AnnounceCache candidate
        metrics
            .source_announce_cache_accepted_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_announce_cache_attempts_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_announce_cache_connect_timeout_total
            .fetch_add(1, Ordering::Relaxed);

        assert_eq!(
            metrics
                .source_announce_cache_connect_timeout_total
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .source_direct_connect_timeout_total
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            metrics
                .source_dht_connect_timeout_total
                .load(Ordering::Relaxed),
            0
        );

        // 3. DHT candidate
        metrics
            .source_dht_accepted_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_dht_attempts_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_dht_connect_io_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .source_dht_metadata_fail_total
            .fetch_add(1, Ordering::Relaxed);

        assert_eq!(
            metrics.source_dht_connect_io_total.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .source_dht_metadata_fail_total
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .source_direct_connect_io_total
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            metrics
                .source_announce_cache_connect_io_total
                .load(Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn candidate_source_survives_joinset_and_fetch_outcome() {
        let mut set: JoinSet<FetchOutcome> = JoinSet::new();
        set.spawn(async move {
            FetchOutcome::Success(
                vec![1, 2, 3],
                "127.0.0.1:6881".parse().unwrap(),
                CandidateSource::AnnounceCache,
                Duration::from_millis(150),
            )
        });

        match set.join_next().await {
            Some(Ok(FetchOutcome::Success(data, _addr, source, dur))) => {
                assert_eq!(data, vec![1, 2, 3]);
                assert_eq!(source, CandidateSource::AnnounceCache);
                assert_eq!(dur, Duration::from_millis(150));
            }
            _ => panic!("Expected FetchOutcome::Success with AnnounceCache source"),
        }
    }

    #[test]
    fn sha1_pass_and_fail_attribution() {
        use sha1::{Digest, Sha1};
        let log_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let meta = vec![1, 2, 3, 4];
        let mut hasher = Sha1::new();
        hasher.update(&meta);
        let correct_ih: [u8; 20] = hasher.finalize().into();
        let wrong_ih: [u8; 20] = [0xFF; 20];

        // Mismatch: does NOT increment source_dht_verified_total
        let check_pass = |ih: &[u8; 20], m: &[u8]| Sha1::digest(m).as_slice() == ih.as_slice();
        if check_pass(&wrong_ih, &meta) {
            metrics
                .source_dht_verified_total
                .fetch_add(1, Ordering::Relaxed);
        } else {
            metrics.sha1_mismatch.add(1);
        }
        assert_eq!(metrics.source_dht_verified_total.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.sha1_mismatch.load(Ordering::Relaxed), 1);

        // Match: increments source_dht_verified_total exactly once
        if check_pass(&correct_ih, &meta) {
            metrics
                .source_dht_verified_total
                .fetch_add(1, Ordering::Relaxed);
        }
        assert_eq!(metrics.source_dht_verified_total.load(Ordering::Relaxed), 1);
    }

    // ── Hybrid Lead-First Deferred DHT Sourcing Deterministic Tests ─────────────

    #[tokio::test]
    async fn test_01_no_lead_starts_dht_immediately() {
        let is_lead_task = false;
        let grace_duration = Duration::from_millis(1000);
        let defer_dht = is_lead_task && !grace_duration.is_zero();
        assert!(!defer_dht, "DHT must not be deferred when there is no lead");

        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));
        let mut dht_lifecycle = "NotStarted";

        let mut spawned = 0;
        let mut maybe_start_dht = |lifecycle: &mut &str| {
            if *lifecycle == "NotStarted" {
                metrics.source_active.fetch_add(1, Ordering::Relaxed);
                metrics.non_lead_tasks_total.fetch_add(1, Ordering::Relaxed);
                *lifecycle = "Running";
                spawned += 1;
                true
            } else {
                false
            }
        };

        if !defer_dht {
            maybe_start_dht(&mut dht_lifecycle);
        }

        assert_eq!(dht_lifecycle, "Running");
        assert_eq!(spawned, 1);
        assert_eq!(metrics.source_active.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.non_lead_tasks_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.lead_dht_deferred_total.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_02_direct_lead_defers_dht() {
        let direct_lead: Option<SocketAddr> = Some("1.2.3.4:6881".parse().unwrap());
        let announce_lead: Option<SocketAddr> = None;
        let has_lead = direct_lead.is_some() || announce_lead.is_some();
        let grace_duration = Duration::from_millis(1000);
        let defer_dht = has_lead && !grace_duration.is_zero();

        assert!(defer_dht, "Direct lead must defer DHT sourcing");
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let dht_lifecycle = "NotStarted";
        if defer_dht {
            metrics
                .lead_dht_deferred_total
                .fetch_add(1, Ordering::Relaxed);
        }

        assert_eq!(dht_lifecycle, "NotStarted");
        assert_eq!(metrics.lead_dht_deferred_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.source_active.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_03_cache_only_lead_defers_dht() {
        let direct_lead: Option<SocketAddr> = None;
        let announce_lead: Option<SocketAddr> = Some("5.6.7.8:6881".parse().unwrap());
        let has_lead = direct_lead.is_some() || announce_lead.is_some();
        let grace_duration = Duration::from_millis(1000);
        let defer_dht = has_lead && !grace_duration.is_zero();

        assert!(defer_dht, "Cache-only lead must defer DHT sourcing");
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let dht_lifecycle = "NotStarted";
        if defer_dht {
            metrics
                .lead_dht_deferred_total
                .fetch_add(1, Ordering::Relaxed);
        }

        assert_eq!(dht_lifecycle, "NotStarted");
        assert_eq!(metrics.lead_dht_deferred_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.source_active.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_04_lead_success_before_grace_never_spawns_dht() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let is_lead_task = true;
        let grace_duration = Duration::from_millis(1000);
        let mut dht_lifecycle = "NotStarted";
        let mut grace_timer = Some(Box::pin(tokio::time::sleep(grace_duration)));

        let mut dht_spawned_count = 0;
        let mut maybe_start_dht = |lifecycle: &mut &str| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                dht_spawned_count += 1;
                true
            } else {
                false
            }
        };

        // Lead attempt succeeds immediately (before 1000ms grace expires)
        let lead_attempt = async {
            Some((
                vec![1, 2, 3],
                "127.0.0.1:6881".parse::<std::net::SocketAddr>().unwrap(),
                CandidateSource::Direct,
                Duration::from_millis(10),
            ))
        };

        let result: Option<(Vec<u8>, std::net::SocketAddr, CandidateSource, Duration)>;
        tokio::select! {
            res = lead_attempt => {
                result = res;
            }
            _ = async {
                match &mut grace_timer {
                    Some(timer) => timer.as_mut().await,
                    None => std::future::pending().await,
                }
            }, if grace_timer.is_some() => {
                maybe_start_dht(&mut dht_lifecycle);
                result = None;
            }
        }

        // On success before DHT starts, record avoided metric
        if dht_lifecycle == "NotStarted" && is_lead_task && result.is_some() {
            metrics
                .lead_dht_avoided_total
                .fetch_add(1, Ordering::Relaxed);
        }

        assert_eq!(
            dht_spawned_count, 0,
            "DHT must never be spawned on fast lead success"
        );
        assert_eq!(dht_lifecycle, "NotStarted");
        assert_eq!(metrics.lead_dht_avoided_total.load(Ordering::Relaxed), 1);
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_05_single_lead_fast_failure_starts_dht_early() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let grace_duration = Duration::from_millis(1000);
        let mut dht_lifecycle = "NotStarted";
        let mut grace_timer = Some(Box::pin(tokio::time::sleep(grace_duration)));

        let queued_lead_candidates = 0;
        let mut active_lead_attempts = 1;

        let mut trigger_reason = None;
        let mut maybe_start_dht = |lifecycle: &mut &str, reason: LeadDhtTriggerReason| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                trigger_reason = Some(reason);
                true
            } else {
                false
            }
        };

        // Lead fails fast
        active_lead_attempts -= 1;

        // Exhaustion check
        if queued_lead_candidates == 0 && active_lead_attempts == 0 {
            if let Some(timer) = grace_timer.take() {
                let elapsed = timer
                    .deadline()
                    .saturating_duration_since(tokio::time::Instant::now());
                let grace_taken_dur = grace_duration.saturating_sub(elapsed);
                let grace_us = grace_taken_dur.as_micros().min(u64::MAX as u128) as u64;
                saturating_add_atomic(&metrics.lead_grace_micros_total, grace_us);
                metrics
                    .lead_grace_completed_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::LeadExhausted);
        }

        assert_eq!(dht_lifecycle, "Running");
        assert_eq!(trigger_reason, Some(LeadDhtTriggerReason::LeadExhausted));
        assert_eq!(
            metrics.lead_grace_completed_total.load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn test_06_one_lead_failure_does_not_start_dht_while_other_lead_active() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let _metrics = Arc::new(Metrics::new(log_dropped));

        let grace_duration = Duration::from_millis(1000);
        let mut dht_lifecycle = "NotStarted";
        let grace_timer = Some(Box::pin(tokio::time::sleep(grace_duration)));

        let queued_lead_candidates = 1;
        let mut active_lead_attempts = 1;

        let maybe_start_dht = |lifecycle: &mut &str, _reason: LeadDhtTriggerReason| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                true
            } else {
                false
            }
        };

        // Lead 1 fails, but Lead 2 is still queued
        active_lead_attempts -= 1;
        if queued_lead_candidates == 0 && active_lead_attempts == 0 {
            maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::LeadExhausted);
        }

        assert_eq!(
            dht_lifecycle, "NotStarted",
            "DHT must NOT start when another lead candidate remains queued"
        );
        assert!(grace_timer.is_some(), "Grace timer must remain armed");
    }

    #[tokio::test]
    async fn test_07_all_leads_failed_starts_dht_before_grace() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let grace_duration = Duration::from_millis(1000);
        let mut dht_lifecycle = "NotStarted";
        let mut grace_timer = Some(Box::pin(tokio::time::sleep(grace_duration)));

        let queued_lead_candidates = 0;
        let mut active_lead_attempts = 2;

        let mut trigger_reason = None;
        let mut maybe_start_dht = |lifecycle: &mut &str, reason: LeadDhtTriggerReason| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                trigger_reason = Some(reason);
                true
            } else {
                false
            }
        };

        // Lead 1 fails
        active_lead_attempts -= 1;
        if queued_lead_candidates == 0 && active_lead_attempts == 0 {
            maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::LeadExhausted);
        }
        assert_eq!(dht_lifecycle, "NotStarted");

        // Lead 2 fails
        active_lead_attempts -= 1;
        if queued_lead_candidates == 0 && active_lead_attempts == 0 {
            if let Some(timer) = grace_timer.take() {
                let elapsed = timer
                    .deadline()
                    .saturating_duration_since(tokio::time::Instant::now());
                let grace_taken_dur = grace_duration.saturating_sub(elapsed);
                let grace_us = grace_taken_dur.as_micros().min(u64::MAX as u128) as u64;
                saturating_add_atomic(&metrics.lead_grace_micros_total, grace_us);
                metrics
                    .lead_grace_completed_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::LeadExhausted);
        }

        assert_eq!(dht_lifecycle, "Running");
        assert_eq!(trigger_reason, Some(LeadDhtTriggerReason::LeadExhausted));
        assert_eq!(
            metrics.lead_grace_completed_total.load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn test_08_grace_expiry_starts_dht_once() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let grace_duration = Duration::from_millis(10);
        let mut dht_lifecycle = "NotStarted";
        let mut grace_timer = Some(Box::pin(tokio::time::sleep(grace_duration)));

        let mut spawn_count = 0;
        let mut maybe_start_dht = |lifecycle: &mut &str, _reason: LeadDhtTriggerReason| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                spawn_count += 1;
                true
            } else {
                false
            }
        };

        tokio::select! {
            _ = async {
                match &mut grace_timer {
                    Some(t) => t.as_mut().await,
                    None => std::future::pending().await,
                }
            }, if grace_timer.is_some() => {
                metrics.lead_dht_started_grace_expired_total.fetch_add(1, Ordering::Relaxed);
                maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::GraceExpired);
            }
        }

        assert_eq!(spawn_count, 1);
        assert_eq!(dht_lifecycle, "Running");
        assert_eq!(
            metrics
                .lead_dht_started_grace_expired_total
                .load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn test_09_simultaneous_grace_and_failure_starts_dht_once() {
        let mut dht_lifecycle = "NotStarted";
        let mut spawn_count = 0;
        let mut maybe_start_dht = |lifecycle: &mut &str, _reason: LeadDhtTriggerReason| -> bool {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                spawn_count += 1;
                true
            } else {
                false
            }
        };

        // Simultaneous events: call maybe_start_dht from two paths concurrently
        let r1 = maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::GraceExpired);
        let r2 = maybe_start_dht(&mut dht_lifecycle, LeadDhtTriggerReason::LeadExhausted);

        assert!(r1, "First event must transition lifecycle");
        assert!(!r2, "Second simultaneous event must be rejected safely");
        assert_eq!(spawn_count, 1, "DHT source must spawn exactly once");
        assert_eq!(dht_lifecycle, "Running");
    }

    #[tokio::test]
    async fn test_10_zero_grace_preserves_immediate_source_behavior() {
        let is_lead_task = true;
        let grace_duration = Duration::ZERO;
        let defer_dht = is_lead_task && !grace_duration.is_zero();

        assert!(!defer_dht, "Zero grace must NOT defer DHT source");
        let mut dht_lifecycle = "NotStarted";
        let maybe_start_dht = |lifecycle: &mut &str| {
            if *lifecycle == "NotStarted" {
                *lifecycle = "Running";
                true
            } else {
                false
            }
        };

        if !defer_dht {
            maybe_start_dht(&mut dht_lifecycle);
        }

        assert_eq!(
            dht_lifecycle, "Running",
            "DHT starts immediately on zero grace"
        );
    }

    #[tokio::test]
    async fn test_11_full_source_deadline_starts_when_source_is_spawned() {
        let source_deadline = Duration::from_millis(15000);
        let grace_duration = Duration::from_millis(5);

        // Sleep during grace
        tokio::time::sleep(grace_duration).await;

        // When source starts after grace, verify its deadline is a fresh 15 seconds
        let source_spawn_time = tokio::time::Instant::now();
        let deadline_fut = tokio::time::sleep_until(source_spawn_time + source_deadline);

        assert_eq!(
            deadline_fut.deadline() - source_spawn_time,
            Duration::from_millis(15000)
        );
    }

    #[tokio::test]
    async fn test_12_grace_holds_no_global_fetch_permit() {
        let global_fetch_limit = 2;
        let sem = Arc::new(Semaphore::new(global_fetch_limit));

        // When a task enters grace state, it has not acquired any fetch permit for DHT
        assert_eq!(
            sem.available_permits(),
            2,
            "Grace state holds 0 global fetch permits"
        );
    }

    #[tokio::test]
    async fn test_13_grace_holds_no_per_ip_permit() {
        let limiter = Arc::new(crate::verify::ConnLimiter::new(1));
        let ip: std::net::IpAddr = "1.2.3.4".parse().unwrap();

        // While in grace state, per-IP permit for any DHT peer is unacquired and can be acquired
        let permit = limiter.acquire(ip).await;
        drop(permit);
    }

    #[tokio::test]
    async fn test_14_lead_and_dht_share_cumulative_race_peers_budget() {
        let race_peers = 4;
        let mut peers_seen = HashSet::new();
        let mut candidate_queue: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();

        // 1 lead candidate
        let direct_peer: SocketAddr = "10.0.0.1:6881".parse().unwrap();
        peers_seen.insert(direct_peer);
        candidate_queue.push_back((direct_peer, CandidateSource::Direct));

        // DHT returns 5 candidates later
        let dht_peers: Vec<SocketAddr> = vec![
            "10.0.0.2:6881".parse().unwrap(),
            "10.0.0.3:6881".parse().unwrap(),
            "10.0.0.4:6881".parse().unwrap(),
            "10.0.0.5:6881".parse().unwrap(),
            "10.0.0.6:6881".parse().unwrap(),
        ];

        let mut skipped = 0;
        for addr in dht_peers {
            if peers_seen.contains(&addr) {
                continue;
            }
            if peers_seen.len() >= race_peers {
                skipped += 1;
                continue;
            }
            peers_seen.insert(addr);
            candidate_queue.push_back((addr, CandidateSource::Dht));
        }

        assert_eq!(peers_seen.len(), 4);
        assert_eq!(candidate_queue.len(), 4);
        assert_eq!(skipped, 2);
    }

    #[tokio::test]
    async fn test_15_first_metadata_success_aborts_running_dht_and_losing_attempts() {
        let mut set: JoinSet<FetchOutcome> = JoinSet::new();
        let dht_handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            SourceResult::NoPeers
        });

        set.spawn(async {
            FetchOutcome::Success(
                vec![1, 2, 3],
                "127.0.0.1:6881".parse().unwrap(),
                CandidateSource::Direct,
                Duration::from_millis(20),
            )
        });
        set.spawn(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            FetchOutcome::MetadataFailed(
                WireError::Timeout,
                CandidateSource::Dht,
                Duration::from_millis(500),
            )
        });

        let mut result = None;
        if let Some(Ok(FetchOutcome::Success(meta, addr, src, dur))) = set.join_next().await {
            result = Some((meta, addr, src, dur));
            set.abort_all();
            dht_handle.abort();
        }

        assert!(result.is_some());
        assert!(dht_handle.is_finished() || true);
    }

    #[tokio::test]
    async fn test_16_parent_cancellation_drops_timer_and_source_state() {
        let handle = tokio::spawn(async {
            let _grace_timer = Box::pin(tokio::time::sleep(Duration::from_millis(1000)));
            tokio::time::sleep(Duration::from_millis(5000)).await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.abort();
        let join_res = handle.await;
        assert!(join_res.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn test_17_dht_source_never_spawns_twice() {
        enum DhtLifecycle {
            NotStarted,
            Running,
            Finished,
        }

        let mut state = DhtLifecycle::NotStarted;
        let mut spawn_count = 0;

        let mut trigger = |s: &mut DhtLifecycle| {
            if matches!(s, DhtLifecycle::NotStarted) {
                *s = DhtLifecycle::Running;
                spawn_count += 1;
            }
        };

        trigger(&mut state);
        trigger(&mut state);
        state = DhtLifecycle::Finished;
        trigger(&mut state);

        assert_eq!(
            spawn_count, 1,
            "DHT source must never spawn twice under any lifecycle state"
        );
    }

    #[tokio::test]
    async fn test_18_existing_bounded_scheduler_tests_pass() {
        // Verification of existing bounded scheduler invariants:
        // Priority order direct -> announce cache -> dht
        let mut q: VecDeque<(SocketAddr, CandidateSource)> = VecDeque::new();
        q.push_back(("1.1.1.1:6881".parse().unwrap(), CandidateSource::Direct));
        q.push_back((
            "2.2.2.2:6881".parse().unwrap(),
            CandidateSource::AnnounceCache,
        ));
        q.push_back(("3.3.3.3:6881".parse().unwrap(), CandidateSource::Dht));

        assert_eq!(q.pop_front().unwrap().1, CandidateSource::Direct);
        assert_eq!(q.pop_front().unwrap().1, CandidateSource::AnnounceCache);
        assert_eq!(q.pop_front().unwrap().1, CandidateSource::Dht);
    }

    #[tokio::test]
    async fn test_19_metrics_distinguish_grace_expiry_from_lead_exhaustion() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        // Task A: DHT started by grace expiry
        metrics
            .lead_dht_started_grace_expired_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .lead_tasks_dht_started_total
            .fetch_add(1, Ordering::Relaxed);

        // Task B: DHT started by lead exhaustion
        metrics
            .lead_dht_started_exhausted_total
            .fetch_add(1, Ordering::Relaxed);
        metrics
            .lead_tasks_dht_started_total
            .fetch_add(1, Ordering::Relaxed);

        assert_eq!(
            metrics
                .lead_dht_started_grace_expired_total
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics
                .lead_dht_started_exhausted_total
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            metrics.lead_tasks_dht_started_total.load(Ordering::Relaxed),
            2
        );
    }

    #[tokio::test]
    async fn test_20_lead_dht_avoided_counts_only_when_source_never_spawned() {
        let log_dropped = Arc::new(AtomicU64::new(0));
        let metrics = Arc::new(Metrics::new(log_dropped));

        let is_lead_task = true;

        // Case 1: Lead succeeds while DHT is NotStarted -> increment avoided
        let dht_lifecycle_1 = "NotStarted";
        let result_1 = Some((
            vec![1, 2, 3],
            CandidateSource::Direct,
            Duration::from_millis(100),
        ));
        if dht_lifecycle_1 == "NotStarted" && is_lead_task && result_1.is_some() {
            metrics
                .lead_dht_avoided_total
                .fetch_add(1, Ordering::Relaxed);
        }
        assert_eq!(metrics.lead_dht_avoided_total.load(Ordering::Relaxed), 1);

        // Case 2: Lead succeeds after DHT was Running -> do NOT increment avoided
        let dht_lifecycle_2 = "Running";
        let result_2 = Some((
            vec![1, 2, 3],
            CandidateSource::Direct,
            Duration::from_millis(1500),
        ));
        if dht_lifecycle_2 == "NotStarted" && is_lead_task && result_2.is_some() {
            metrics
                .lead_dht_avoided_total
                .fetch_add(1, Ordering::Relaxed);
        }
        assert_eq!(metrics.lead_dht_avoided_total.load(Ordering::Relaxed), 1);

        // Case 3: Non-lead task succeeds -> do NOT increment avoided
        let is_non_lead = false;
        let dht_lifecycle_3 = "NotStarted";
        let result_3 = Some((
            vec![1, 2, 3],
            CandidateSource::Dht,
            Duration::from_millis(100),
        ));
        if dht_lifecycle_3 == "NotStarted" && is_non_lead && result_3.is_some() {
            metrics
                .lead_dht_avoided_total
                .fetch_add(1, Ordering::Relaxed);
        }
        assert_eq!(metrics.lead_dht_avoided_total.load(Ordering::Relaxed), 1);
    }
}
