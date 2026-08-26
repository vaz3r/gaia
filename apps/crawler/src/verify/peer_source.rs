use crate::dht::routing_table::{NodeInfo, decode_compact, xor};
use crate::krpc::Infohash;
use crate::krpc::codec::BValue;
use crate::krpc::message::{GET_PEERS, Kind, Message};
use crate::metrics::{Add1, Metrics};
use crate::router::{QueryError, Router};
use bytes::Bytes;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

pub enum SourceResult {
    Peers(Vec<SocketAddr>),
    NoPeers,
    AllTimeout,
}

pub async fn source_peers(
    router: Arc<Router>,
    info_hash: Infohash,
    count: usize,
    metrics: Arc<Metrics>,
    deadline: Duration,
    k: usize,
    alpha: usize,
    query_timeout: Duration,
    max_queries: usize,
) -> SourceResult {
    let k = k.max(1);
    let alpha = alpha.max(1);
    let mut candidates: Vec<NodeInfo> = router.closest_nodes(&info_hash, k);
    if candidates.is_empty() {
        candidates = router.random_routing_nodes(k);
    }

    let mut queried: HashSet<SocketAddr> = HashSet::new();
    let mut seen: HashSet<SocketAddr> = HashSet::new();
    let mut peers: Vec<SocketAddr> = Vec::new();
    let mut succeeded: u64 = 0;

    let start_time = std::time::Instant::now();
    crate::trace_lifecycle!(&info_hash, "source_start", stream = "dht");

    // Pipelined iterative get_peers lookup: keep up to ALPHA queries in
    // flight at all times, issuing the next closest unqueried candidate as
    // soon as any query completes, instead of blocking a whole round on the
    // slowest peer. The global deadline wraps the entire loop, bounding the
    // worst case; on expiry we abort and return whatever peers we found.
    let mut set: JoinSet<(SocketAddr, Result<Message, QueryError>)> = JoinSet::new();
    let mut inflight: usize = 0;
    let mut new_nodes: Vec<NodeInfo> = Vec::new();
    let mut total_queries: usize = 0;

    let lookup = async {
        // Prime the pipeline with up to ALPHA initial queries.
        while inflight < alpha {
            if total_queries >= max_queries {
                break;
            }
            let idx = candidates
                .iter()
                .position(|n| n.addr != router.self_addr && !queried.contains(&n.addr));
            let Some(node) = idx.map(|i| candidates.remove(i)) else {
                break;
            };
            queried.insert(node.addr);
            inflight += 1;
            total_queries += 1;
            spawn_query(&mut set, &router, info_hash, node, &metrics, query_timeout);
        }

        while inflight > 0 {
            match set.join_next().await {
                Some(Ok((node_addr, Ok(msg)))) => {
                    if let Kind::Response { r } = &msg.kind {
                        metrics.source_responses.add(1);
                        succeeded += 1;
                        let mut returned_here = 0;
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
                                    returned_here += 1;
                                    if seen.insert(addr) {
                                        peers.push(addr);
                                        metrics.source_peers_returned.add(1);
                                    }
                                }
                            }
                        }
                        crate::trace_lifecycle!(
                            &info_hash,
                            "source_response",
                            stream = "dht",
                            node = node_addr.to_string(),
                            peers_returned = returned_here
                        );
                        if let Some(nodes) = r.get_bytes(b"nodes") {
                            new_nodes.extend(decode_compact(nodes));
                        }
                    }
                }
                Some(Ok((_, Err(_)))) | Some(Err(_)) => {
                    metrics.source_timeout.add(1);
                }
                None => break,
            }
            inflight -= 1;

            if !new_nodes.is_empty() {
                candidates.extend(new_nodes.drain(..));
                candidates.sort_by_key(|n| xor(&info_hash, &n.id));
                candidates.dedup_by(|a, b| a.id == b.id);
                candidates.truncate(k);
            }

            if peers.len() >= count {
                set.abort_all();
                break;
            }

            while inflight < alpha {
                if total_queries >= max_queries {
                    break;
                }
                let idx = candidates
                    .iter()
                    .position(|n| n.addr != router.self_addr && !queried.contains(&n.addr));
                let Some(node) = idx.map(|i| candidates.remove(i)) else {
                    break;
                };
                queried.insert(node.addr);
                inflight += 1;
                total_queries += 1;
                spawn_query(&mut set, &router, info_hash, node, &metrics, query_timeout);
            }
        }
    };

    let timed = tokio::time::timeout(deadline, lookup).await;
    if timed.is_err() {
        metrics.source_deadline_hits.add(1);
        metrics.source_deadline_peers.add(peers.len() as u64);
        set.abort_all();
    }

    peers.truncate(count);
    metrics.source_returned_peers.add(peers.len() as u64);
    if peers.is_empty() {
        metrics.source_no_values.add(1);
    }
    crate::trace_lifecycle!(
        &info_hash,
        "source_done",
        stream = "dht",
        peers = peers.len(),
        elapsed_ms = start_time.elapsed().as_millis() as u64
    );
    if succeeded == 0 {
        metrics.source_all_timeout.add(1);
        SourceResult::AllTimeout
    } else if peers.is_empty() {
        SourceResult::NoPeers
    } else {
        SourceResult::Peers(peers)
    }
}

fn spawn_query(
    set: &mut JoinSet<(SocketAddr, Result<Message, QueryError>)>,
    router: &Arc<Router>,
    info_hash: Infohash,
    node: NodeInfo,
    metrics: &Arc<Metrics>,
    query_timeout: Duration,
) {
    let router = router.clone();
    let ih = info_hash;
    let addr = node.addr;
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
    set.spawn(async move {
        let node_str = addr.to_string();
        crate::trace_lifecycle!(&ih, "source_query", stream = "dht", node = node_str);
        let res = router
            .send_query(GET_PEERS, addr, args, query_timeout)
            .await;
        (addr, res)
    });
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
