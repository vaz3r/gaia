pub mod fetch_pool;
pub mod peer_cache;
pub mod peer_source;
#[allow(clippy::module_inception)]
pub mod verify;
pub mod wire;

use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::storage::batch_writer::BatchWriter;
use crate::verify::fetch_pool::{VerifyResult, verify_infohash};
use crate::verify::peer_cache::PeerCache;
use crate::verify::verify::check;
use librqbit_utp::UtpSocketUdp;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Semaphore, mpsc};

pub struct VerifyConfig {
    pub global_limit: usize,
    pub race_peers: usize,
}

pub async fn run_pipeline(
    mut rx: mpsc::Receiver<Infohash>,
    mut announce_rx: mpsc::Receiver<(Infohash, SocketAddr)>,
    node_routers: Arc<Vec<Arc<Router>>>,
    utp: Option<Arc<UtpSocketUdp>>,
    metrics: Arc<Metrics>,
    batch_writer: Arc<BatchWriter>,
    peer_cache: Arc<PeerCache>,
    config: VerifyConfig,
) {
    let global = Arc::new(Semaphore::new(config.global_limit.max(1)));
    let next_router = AtomicUsize::new(0);
    let race = config.race_peers.max(1);
    loop {
        let Ok(_permit) = global.clone().acquire_owned().await else {
            break;
        };

        let (ih, direct) = tokio::select! {
            item = announce_rx.recv() => {
                match item {
                    Some((ih, addr)) => (ih, Some(addr)),
                    None => match rx.recv().await {
                        Some(ih) => (ih, None),
                        None => break,
                    },
                }
            }
            item = rx.recv() => {
                match item {
                    Some(ih) => (ih, None),
                    None => match announce_rx.recv().await {
                        Some((ih, addr)) => (ih, Some(addr)),
                        None => break,
                    },
                }
            }
        };
        let router = node_routers[next_router.fetch_add(1, Ordering::Relaxed) % node_routers.len()].clone();
        let is_direct = direct.is_some();
        let utp = utp.clone();
        let metrics = metrics.clone();
        let batch_writer = batch_writer.clone();
        let peer_cache = peer_cache.clone();
        tokio::spawn(async move {
            let _permit = _permit;
            if is_direct {
                metrics.announce_attempts.add(1);
            }
            metrics.verify_attempts.add(1);
            match verify_infohash(router, utp, ih, race, metrics.clone(), peer_cache, direct).await {
                VerifyResult::Success(meta) if check(&ih, &meta) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", result = "pass");
                    metrics.verify_success.add(1);
                    if is_direct {
                        metrics.announce_success.add(1);
                    }
                    batch_writer.push_torrent(ih, &meta);
                    crate::trace_lifecycle!(&ih, "persist_torrents", status = "ok");
                    batch_writer.push_verified(ih);
                }
                VerifyResult::Success(_) => {
                    crate::trace_lifecycle!(&ih, "sha1_check", result = "fail");
                    metrics.sha1_mismatch.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "sha1_mismatch");
                }
                VerifyResult::NoPeers => {
                    crate::trace_lifecycle!(&ih, "verify_fail", result = "no_peers");
                    metrics.source_no_peers.add(1);
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_peers");
                }
                VerifyResult::SourceTimeout => {
                    crate::trace_lifecycle!(&ih, "verify_fail", result = "source_timeout");
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "source_timeout");
                }
                VerifyResult::MetadataFailed => {
                    crate::trace_lifecycle!(&ih, "verify_fail", result = "no_metadata");
                    metrics.verify_fail.add(1);
                    batch_writer.push_failed(ih, "no_metadata");
                }
            }
        });
    }
}
