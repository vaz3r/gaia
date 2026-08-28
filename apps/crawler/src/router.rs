use crate::dht::node_id::SybilPool;
use crate::dht::routing_table::{NodeInfo, RoutingTable, encode_compact, cmp_xor, xor};
use crate::harvest::{HarvestEvent, Source};
use crate::krpc::codec::BValue;
use crate::krpc::message::{ANNOUNCE_PEER, FIND_NODE, GET_PEERS, Kind, Message, PING};
use crate::krpc::token::TokenGenerator;
use crate::krpc::tx_state::{TxEntry, TxKind, TxTable};
use crate::krpc::{Infohash, NodeId};
use crate::metrics::{Add1, Metrics};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub enum QueryError {
    Timeout,
    Cancelled,
    Io(std::io::Error),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Timeout => write!(f, "query timed out"),
            QueryError::Cancelled => write!(f, "query cancelled"),
            QueryError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for QueryError {}

pub struct Router {
    pub self_id: NodeId,
    pub self_addr: SocketAddr,
    pub sybils: Vec<(NodeId, SybilPool)>,
    pub external_ip: Option<std::net::IpAddr>,
    send_socks: Arc<Vec<Arc<UdpSocket>>>,
    send_idx: AtomicUsize,
    tx: Arc<TxTable>,
    token: Arc<RwLock<TokenGenerator>>,
    table: Arc<Mutex<RoutingTable>>,
    harvest_tx: mpsc::Sender<HarvestEvent>,
    metrics: Arc<Metrics>,
    find_node_response_percent: u8,
}

impl Router {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        self_id: NodeId,
        self_addr: SocketAddr,
        sybils: Vec<(NodeId, SybilPool)>,
        external_ip: Option<std::net::IpAddr>,
        send_socks: Arc<Vec<Arc<UdpSocket>>>,
        tx: Arc<TxTable>,
        token: Arc<RwLock<TokenGenerator>>,
        table: Arc<Mutex<RoutingTable>>,
        harvest_tx: mpsc::Sender<HarvestEvent>,
        metrics: Arc<Metrics>,
    find_node_response_percent: u8,
    ) -> Arc<Self> {
        Arc::new(Router {
            self_id,
            self_addr,
            sybils,
            external_ip,
            send_socks,
            send_idx: AtomicUsize::new(0),
            tx,
            token,
            table,
            harvest_tx,
            metrics,
            find_node_response_percent,
        })
    }

    pub fn handle_datagram(&self, buf: &[u8], from: SocketAddr) {
        let header = match crate::krpc::scanner::scan(buf) {
            Some(h) => h,
            None => {
                self.metrics.inbound_invalid.add(1);
                return;
            }
        };

        if let Some(y) = header.y {
            if y == b"q" {
                if let Some(q) = header.q {
                    if q == PING {
                        self.metrics.inbound_ping.add(1);
                        if let Some(t) = header.t {
                            self.respond_ping_fast(t, from);
                        }
                        return;
                    }
                    if q == GET_PEERS {
                        self.metrics.inbound_get_peers.add(1);
                        if let (Some(t), Some(ih)) = (header.t, Self::extract_info_hash(buf)) {
                            self.respond_get_peers_fast(t, &ih, from);
                        }
                        return;
                    }
                    if q == FIND_NODE {
                        if self.find_node_response_percent < 100
                            && !crate::router::should_answer(self.find_node_response_percent, rand::random::<u16>())
                        {
                            self.metrics.inbound_find_node_dropped.add(1);
                            return;
                        }
                    }
                }
            } else if y == b"r" || y == b"e" {
                if let Some(t) = header.t {
                    if let Some(entry) = self.tx.take(t) {
                        if let Some(reply) = entry.reply {
                            let _ = reply.send(Bytes::copy_from_slice(buf));
                        }
                    }
                }
                return;
            }
        }

        let bytes = Bytes::copy_from_slice(buf);
        let msg = match Message::parse(&bytes) {
            Ok(m) => m,
            Err(_) => {
                self.metrics.inbound_invalid.add(1);
                return;
            }
        };
        match &msg.kind {
            Kind::Query { q, a } => self.handle_query(&msg.t, q, a, from),
            _ => {} // Handled by fast path above
        }
    }

    fn handle_query(&self, t: &Bytes, q: &Bytes, a: &BValue, from: SocketAddr) {
        match q.as_ref() {
            PING => {
                self.metrics.inbound_ping.add(1);
                self.send_response(t, from, self.id_response());
            }
            FIND_NODE => {
                self.metrics.inbound_find_node.add(1);
                self.respond_find_node(t, a, from);
            }
            GET_PEERS => {
                self.metrics.inbound_get_peers.add(1);
                self.respond_get_peers(t, a, from);
            }
            ANNOUNCE_PEER => {
                self.metrics.inbound_announce_peer.add(1);
                self.respond_announce_peer(t, a, from);
            }
            _ => {}
        }
    }

    fn id_response(&self) -> BValue {
        BValue::dict(vec![(
            Bytes::from_static(b"id"),
            BValue::Bytes(Bytes::copy_from_slice(&self.self_id)),
        )])
    }

    fn closest_phantom(&self, target: &[u8; 20], count: usize) -> Vec<NodeInfo> {
        let public_addr = SocketAddr::new(
            self.external_ip.unwrap_or(self.self_addr.ip()),
            self.self_addr.port(),
        );
        let mut sybils: Vec<NodeInfo> = self
            .sybils
            .iter()
            .map(|(id, _)| NodeInfo {
                id: *id,
                addr: public_addr,
            })
            .collect();
        sybils.sort_unstable_by(|a, b| cmp_xor(target, &a.id, &b.id));
        sybils.truncate(count);
        if sybils.len() < count {
            let known = self
                .table
                .lock()
                .expect("routing table poisoned")
                .closest(target, count - sybils.len());
            sybils.extend(known);
        }
        sybils
    }

    fn classify_pool(&self, target: &[u8; 20]) -> SybilPool {
        self.sybils
            .iter()
            .min_by_key(|(id, _)| xor(target, id))
            .map(|(_, pool)| *pool)
            .unwrap_or(SybilPool::Random)
    }

    fn respond_find_node(&self, t: &Bytes, a: &BValue, from: SocketAddr) {
        let target = match extract_id20(a, b"target") {
            Some(t) => t,
            None => return,
        };
        match self.classify_pool(&target) {
            SybilPool::Bep42 => self.metrics.inbound_find_node_bep42.add(1),
            SybilPool::Random => self.metrics.inbound_find_node_random.add(1),
        }

        let nodes = self.closest_phantom(&target, 8);
        let r = BValue::dict(vec![
            (
                Bytes::from_static(b"id"),
                BValue::Bytes(Bytes::copy_from_slice(&self.self_id)),
            ),
            (
                Bytes::from_static(b"nodes"),
                BValue::Bytes(Bytes::from(encode_compact(&nodes))),
            ),
        ]);
        self.send_response(t, from, r);
    }

    fn respond_get_peers(&self, t: &Bytes, a: &BValue, from: SocketAddr) {
        let ih = match extract_id20(a, b"info_hash") {
            Some(ih) => ih,
            None => return,
        };
        match self.classify_pool(&ih) {
            SybilPool::Bep42 => self.metrics.inbound_get_peers_bep42.add(1),
            SybilPool::Random => self.metrics.inbound_get_peers_random.add(1),
        }
        self.do_harvest(ih, crate::harvest::Source::GetPeers, None);
        let token = self
            .token
            .read()
            .expect("token generator poisoned")
            .generate(from.ip());
        self.metrics.tokens_issued.add(1);
        let nodes = self.closest_phantom(&ih, 8);
        let r = BValue::dict(vec![
            (
                Bytes::from_static(b"id"),
                BValue::Bytes(Bytes::copy_from_slice(&self.self_id)),
            ),
            (
                Bytes::from_static(b"token"),
                BValue::Bytes(Bytes::copy_from_slice(&token)),
            ),
            (
                Bytes::from_static(b"nodes"),
                BValue::Bytes(Bytes::from(encode_compact(&nodes))),
            ),
        ]);
        self.send_response(t, from, r);
    }

    fn respond_announce_peer(&self, t: &Bytes, a: &BValue, from: SocketAddr) {
        let valid = a
            .get_bytes(b"token")
            .map(|tok| self.token.read().expect("token").verify(from.ip(), tok))
            .unwrap_or(false);
        if let Some(ih) = extract_id20(a, b"info_hash") {
            match self.classify_pool(&ih) {
                SybilPool::Bep42 => self.metrics.inbound_announce_bep42.add(1),
                SybilPool::Random => self.metrics.inbound_announce_random.add(1),
            }
            if valid {
                self.metrics.inbound_announce_valid.add(1);
                let implied = a.get_int(b"implied_port").unwrap_or(0) != 0;
                let target_port = if implied {
                    from.port()
                } else {
                    a.get_int(b"port")
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|&p| p != 0)
                        .unwrap_or(from.port())
                };
                let peer_addr = SocketAddr::new(from.ip(), target_port);
                self.do_harvest(ih, crate::harvest::Source::AnnouncePeer, Some(peer_addr));
            } else {
                self.metrics.inbound_announce_invalid_token.add(1);
            }
        }
        let r = self.id_response();
        self.send_response(t, from, r);
    }

    fn do_harvest(&self, ih: Infohash, source: Source, direct: Option<SocketAddr>) {
        self.metrics.infohashes_harvested.add(1);
        if self
            .harvest_tx
            .try_send(HarvestEvent {
                ih,
                source,
                direct,
            })
            .is_err()
        {
            self.metrics.harvest_try_send_dropped.add(1);
        }
    }



    fn extract_info_hash(buf: &[u8]) -> Option<[u8; 20]> {
        let target = b"9:info_hash20:";
        if let Some(pos) = buf.windows(target.len()).position(|w| w == target) {
            let start = pos + target.len();
            if start + 20 <= buf.len() {
                let mut ih = [0u8; 20];
                ih.copy_from_slice(&buf[start..start+20]);
                return Some(ih);
            }
        }
        None
    }

    fn respond_get_peers_fast(&self, t: &[u8], ih: &[u8; 20], from: SocketAddr) {
        use std::io::Write;
        match self.classify_pool(ih) {
            SybilPool::Bep42 => self.metrics.inbound_get_peers_bep42.add(1),
            SybilPool::Random => self.metrics.inbound_get_peers_random.add(1),
        }
        self.do_harvest(*ih, crate::harvest::Source::GetPeers, None);
        let token = self.token.read().expect("token").generate(from.ip());
        self.metrics.tokens_issued.add(1);
        
        let nodes = self.closest_phantom(ih, 8);
        let compact = crate::dht::routing_table::encode_compact(&nodes);
        
        let mut buf = [0u8; 512];
        let mut pos = 0;
        
        let b1 = b"d1:rd2:id20:";
        buf[pos..pos+b1.len()].copy_from_slice(b1);
        pos += b1.len();
        
        buf[pos..pos+20].copy_from_slice(&self.self_id);
        pos += 20;
        
        let b2 = b"5:nodes";
        buf[pos..pos+b2.len()].copy_from_slice(b2);
        pos += b2.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", compact.len()).unwrap();
        pos += cursor.position() as usize;
        
        buf[pos..pos+compact.len()].copy_from_slice(&compact);
        pos += compact.len();
        
        let b3 = b"5:token8:";
        buf[pos..pos+b3.len()].copy_from_slice(b3);
        pos += b3.len();
        
        buf[pos..pos+8].copy_from_slice(&token);
        pos += 8;
        
        let b4 = b"e1:t";
        buf[pos..pos+b4.len()].copy_from_slice(b4);
        pos += b4.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", t.len()).unwrap();
        pos += cursor.position() as usize;
        
        buf[pos..pos+t.len()].copy_from_slice(t);
        pos += t.len();
        
        let b5 = b"1:y1:re";
        buf[pos..pos+b5.len()].copy_from_slice(b5);
        pos += b5.len();
        
        self.try_send(&buf[..pos], from);
    }

    fn respond_ping_fast(&self, t: &[u8], from: SocketAddr) {
        use std::io::Write;
        let mut buf = [0u8; 128];
        let mut pos = 0;
        let b1 = b"d1:rd2:id20:";
        buf[pos..pos+b1.len()].copy_from_slice(b1);
        pos += b1.len();
        buf[pos..pos+20].copy_from_slice(&self.self_id);
        pos += 20;
        let b2 = b"e1:t";
        buf[pos..pos+b2.len()].copy_from_slice(b2);
        pos += b2.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", t.len()).unwrap();
        let written = cursor.position() as usize;
        pos += written;
        
        if pos + t.len() + 7 <= buf.len() {
            buf[pos..pos+t.len()].copy_from_slice(t);
            pos += t.len();
            let b3 = b"1:y1:re";
            buf[pos..pos+b3.len()].copy_from_slice(b3);
            pos += b3.len();
            self.try_send(&buf[..pos], from);
        }
    }

    fn send_response(&self, t: &Bytes, from: SocketAddr, r: BValue) {
        let msg = Message::response(t.clone(), r);
        let enc = msg.encode();
        self.try_send(&enc, from);
    }

    fn next_socket(&self) -> &UdpSocket {
        let idx = self.send_idx.fetch_add(1, AtomicOrdering::Relaxed) % self.send_socks.len();
        &self.send_socks[idx]
    }

    fn try_send(&self, buf: &[u8], addr: SocketAddr) {
        match self.next_socket().try_send_to(buf, addr) {
            Ok(_) => {}
            Err(_) => {
                self.metrics.send_dropped.add(1);
            }
        }
    }

    pub async fn send_query(
        &self,
        method: &[u8],
        addr: SocketAddr,
        args: BValue,
        timeout: Duration,
    ) -> Result<Message, QueryError> {
        let (txid, rx) = self.register(method);
        let msg = Message::query(txid.clone(), method, args);
        let enc = msg.encode();
        self.metrics.outbound_queries.add(1);
        if let Err(e) = self.next_socket().send_to(&enc, addr).await {
            self.tx.take(&txid);
            return Err(QueryError::Io(e));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(bytes)) => match Message::parse(&bytes) {
                Ok(m) => Ok(m),
                Err(_) => Err(QueryError::Cancelled),
            },
            Ok(Err(_)) => Err(QueryError::Cancelled),
            Err(_) => {
                self.tx.take(&txid);
                self.metrics.outbound_timeouts.add(1);
                Err(QueryError::Timeout)
            }
        }
    }

    fn register(&self, method: &[u8]) -> (Bytes, oneshot::Receiver<Bytes>) {
        for _ in 0..32 {
            let txid = Bytes::copy_from_slice(&rand::random::<[u8; 2]>());
            let kind = match method {
                PING => TxKind::Ping,
                FIND_NODE => TxKind::FindNode,
                GET_PEERS => TxKind::GetPeers,
                ANNOUNCE_PEER => TxKind::AnnouncePeer,
                _ => TxKind::Ping,
            };
            let (tx, rx) = oneshot::channel();
            let entry = TxEntry {
                kind,
                sent: Instant::now(),
                reply: Some(tx),
            };
            if self.tx.insert(txid.clone(), entry) {
                return (txid, rx);
            }
        }
        unreachable!("txid space exhausted")
    }

    pub fn insert_nodes(&self, nodes: Vec<NodeInfo>) {
        let mut table = self.table.lock().expect("routing table poisoned");
        self.metrics.routing_insert_calls.add(nodes.len() as u64);
        let mut new_ids = 0u64;
        let mut rejected = 0u64;
        for n in nodes {
            if n.id == self.self_id {
                rejected += 1;
                continue;
            }
            if !table.contains_id(&n.id) {
                new_ids += 1;
            }
            table.insert(n);
        }
        self.metrics.routing_new_ids.add(new_ids);
        self.metrics.routing_rejected.add(rejected);
        self.metrics.routing_table_len.store(table.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.metrics.routing_buckets_used.store(table.buckets_used() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn routing_nodes(&self) -> Vec<NodeInfo> {
        self.table.lock().expect("routing table poisoned").all()
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    pub fn random_routing_nodes(&self, n: usize) -> Vec<NodeInfo> {
        self.table
            .lock()
            .expect("routing table poisoned")
            .random_nodes(n)
    }

    pub fn closest_nodes(&self, target: &[u8; 20], n: usize) -> Vec<NodeInfo> {
        self.table
            .lock()
            .expect("routing table poisoned")
            .closest(target, n)
    }

    pub fn cleanup_tx(&self, ttl: Duration) {
        let removed = self.tx.cleanup(Instant::now(), ttl);
        self.metrics
            .tx_table_len
            .store(self.tx.len() as u64, std::sync::atomic::Ordering::Relaxed);
        let _ = removed;
    }
}

fn extract_id20(a: &BValue, key: &[u8]) -> Option<[u8; 20]> {
    let b = a.get_bytes(key)?;
    if b.len() != 20 {
        return None;
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(b);
    Some(out)
}

fn should_answer(percent: u8, roll: u16) -> bool {
    (roll % 100) < (percent as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_answer() {
        assert!(should_answer(100, 50));
        assert!(should_answer(100, 0));
        assert!(should_answer(100, 99));
        
        assert!(should_answer(5, 4));
        assert!(!should_answer(5, 5));
        assert!(!should_answer(5, 99));

        assert!(should_answer(1, 0));
        assert!(!should_answer(1, 1));
        
        assert!(!should_answer(0, 0));
        assert!(!should_answer(0, 50));
    }
}
