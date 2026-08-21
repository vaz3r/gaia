use crate::dht::routing_table::{NodeInfo, decode_compact, xor};
use crate::krpc::Infohash;
use crate::krpc::codec::BValue;
use crate::krpc::message::{GET_PEERS, Kind};
use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use crate::verify::peer_cache::PeerCache;
use bytes::Bytes;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

const QUERY_TIMEOUT: Duration = Duration::from_secs(8);
const K: usize = 12;
const ALPHA: usize = 3;
const MAX_ROUNDS: usize = 8;

pub async fn source_peers(
    router: Arc<Router>,
    info_hash: Infohash,
    count: usize,
    metrics: Arc<Metrics>,
    peer_cache: Arc<PeerCache>,
) -> Vec<SocketAddr> {
    let mut candidates: Vec<NodeInfo> = router.closest_nodes(&info_hash, K);
    if candidates.is_empty() {
        candidates = router.random_routing_nodes(K);
    }

    let mut queried: HashSet<SocketAddr> = HashSet::new();
    let mut seen: HashSet<SocketAddr> = HashSet::new();
    let mut peers: Vec<SocketAddr> = Vec::new();
    let mut succeeded: u64 = 0;

    for _ in 0..MAX_ROUNDS {
        let batch: Vec<NodeInfo> = candidates
            .iter()
            .filter(|n| n.addr != router.self_addr && !queried.contains(&n.addr))
            .take(ALPHA)
            .cloned()
            .collect();
        if batch.is_empty() {
            break;
        }
        for n in &batch {
            queried.insert(n.addr);
        }

        let mut tasks = Vec::with_capacity(batch.len());
        for node in batch {
            let router = router.clone();
            let args = BValue::dict(vec![
                (
                    Bytes::from_static(b"id"),
                    BValue::Bytes(Bytes::copy_from_slice(&router.self_id)),
                ),
                (
                    Bytes::from_static(b"info_hash"),
                    BValue::Bytes(Bytes::copy_from_slice(&info_hash)),
                ),
            ]);
            metrics.source_queries.add(1);
            tasks.push(tokio::spawn(async move {
                router
                    .send_query(GET_PEERS, node.addr, args, QUERY_TIMEOUT)
                    .await
            }));
        }

        let mut new_nodes: Vec<NodeInfo> = Vec::new();
        for task in tasks {
            match task.await {
                Ok(Ok(msg)) => {
                    let Kind::Response { r } = &msg.kind else {
                        continue;
                    };
                    metrics.source_responses.add(1);
                    succeeded += 1;
                    if let Some(values) = r.get(b"values").and_then(BValue::as_list) {
                        for v in values {
                            if let Some(b) = v.as_bytes()
                                && b.len() == 6
                            {
                                let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
                                if !is_routable(ip) {
                                    continue;
                                }
                                let addr = SocketAddr::new(
                                    std::net::IpAddr::V4(ip),
                                    u16::from_be_bytes([b[4], b[5]]),
                                );
                                if seen.insert(addr) {
                                    peers.push(addr);
                                    metrics.source_peers_returned.add(1);
                                }
                            }
                        }
                    }
                    if let Some(nodes) = r.get_bytes(b"nodes") {
                        new_nodes.extend(decode_compact(nodes));
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    metrics.source_timeout.add(1);
                }
            }
            if peers.len() >= count {
                break;
            }
        }

        if peers.len() >= count {
            break;
        }
        if !new_nodes.is_empty() {
            candidates.extend(new_nodes);
            candidates.sort_by_key(|n| xor(&info_hash, &n.id));
            candidates.dedup_by(|a, b| a.addr == b.addr);
            candidates.truncate(K);
        }
    }

    peers.truncate(count);
    // Track total peers returned before cache filtering
    metrics.source_returned_peers.add(peers.len() as u64);
    // Filter out cached-bad peers
    let filtered: Vec<SocketAddr> = peers
        .into_iter()
        .filter(|addr| {
            if peer_cache.is_bad(addr) {
                metrics.peer_cache_hits.add(1);
                metrics.source_filtered_by_cache.add(1);
                false
            } else {
                true
            }
        })
        .collect();
    // Infohash-level source failure classification
    if succeeded == 0 {
        metrics.source_all_timeout.add(1);
        metrics.verify_fail.add(1);
    } else if filtered.is_empty() {
        metrics.source_no_peers.add(1);
        metrics.verify_fail.add(1);
    }
    filtered
}

fn is_routable(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || o[0] == 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routable_filter() {
        assert!(is_routable(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_routable(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(!is_routable(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(!is_routable(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_routable(Ipv4Addr::new(0, 0, 0, 0)));
        assert!(!is_routable(Ipv4Addr::new(100, 64, 0, 1)));
    }
}
