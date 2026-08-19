pub mod fetch_pool;
pub mod peer_source;
#[allow(clippy::module_inception)]
pub mod verify;
pub mod wire;

use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::storage::jobs::VerifyStore;
use crate::storage::torrents::TorrentStore;
use crate::verify::fetch_pool::verify_infohash;
use crate::verify::verify::check;
use librqbit_utp::UtpSocketUdp;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};

pub struct VerifyConfig {
    pub global_limit: usize,
    pub race_peers: usize,
}

pub async fn run_pipeline(
    mut rx: mpsc::Receiver<Infohash>,
    router: Arc<Router>,
    utp: Option<Arc<UtpSocketUdp>>,
    metrics: Arc<Metrics>,
    torrents: Arc<TorrentStore>,
    jobs: Arc<VerifyStore>,
    config: VerifyConfig,
) {
    let global = Arc::new(Semaphore::new(config.global_limit.max(1)));
    while let Some(ih) = rx.recv().await {
        let permit = match global.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let router = router.clone();
        let utp = utp.clone();
        let metrics = metrics.clone();
        let torrents = torrents.clone();
        let jobs = jobs.clone();
        let race = config.race_peers.max(1);
        tokio::spawn(async move {
            let _permit = permit;
            metrics.verify_attempts.add(1);
            match verify_infohash(router, utp, ih, race, metrics.clone()).await {
                Some(meta) if check(&ih, &meta) => {
                    metrics.verify_success.add(1);
                    if let Err(e) = torrents.store(ih, &meta).await {
                        tracing::warn!(error = %e, "torrent store insert failed");
                    }
                    let _ = jobs.mark_verified(ih).await;
                }
                _ => {
                    metrics.verify_fail.add(1);
                    let _ = jobs.mark_failed(ih, "fetch failed").await;
                }
            }
        });
    }
}
