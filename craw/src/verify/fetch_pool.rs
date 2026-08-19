use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::verify::peer_source::source_peers;
use crate::verify::wire::{WireError, WireSession, gen_peer_id};
use librqbit_utp::UtpSocketUdp;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_TIMEOUT: Duration = Duration::from_secs(25);

pub async fn verify_infohash(
    router: Arc<Router>,
    utp: Option<Arc<UtpSocketUdp>>,
    info_hash: Infohash,
    race_peers: usize,
    metrics: Arc<Metrics>,
) -> Option<Vec<u8>> {
    let peers = source_peers(router, info_hash, race_peers.max(1), metrics.clone()).await;
    if peers.is_empty() {
        return None;
    }

    metrics.fetch_attempts.add(peers.len() as u64);
    let peer_id = gen_peer_id();
    let local = Arc::new(Semaphore::new(race_peers.max(1)));
    let mut set = JoinSet::new();
    for addr in peers {
        let per = local.clone();
        let utp = utp.clone();
        let ih = info_hash;
        let pid = peer_id;
        set.spawn(async move {
            let _permit = per
                .try_acquire()
                .expect("local semaphore is sized to the race");
            let mut session = match WireSession::connect_tcp(addr, &ih, &pid, CONNECT_TIMEOUT).await
            {
                Ok(s) => s,
                Err(tcp_err) => match &utp {
                    Some(sock) => {
                        match WireSession::connect_utp(
                            sock.clone(),
                            addr,
                            &ih,
                            &pid,
                            CONNECT_TIMEOUT,
                        )
                        .await
                        {
                            Ok(s) => s,
                            Err(_) => return Err(tcp_err),
                        }
                    }
                    None => return Err(tcp_err),
                },
            };
            session.fetch_metadata(FETCH_TIMEOUT).await
        });
    }

    let mut result = None;
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(meta)) => {
                result = Some(meta);
                break;
            }
            Ok(Err(WireError::Timeout)) => metrics.verify_timeouts.add(1),
            _ => {}
        }
    }
    set.abort_all();
    result
}
