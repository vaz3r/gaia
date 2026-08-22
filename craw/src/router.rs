use crate::dht::node_id::SybilPool;
use crate::dht::routing_table::{NodeInfo, RoutingTable, encode_compact, xor};
use crate::harvest::Harvester;
use crate::krpc::codec::BValue;
use crate::krpc::message::{ANNOUNCE_PEER, FIND_NODE, GET_PEERS, Kind, Message, PING};
use crate::krpc::token::TokenGenerator;
use crate::krpc::tx_state::{TxEntry, TxKind, TxTable};
use crate::krpc::{Infohash, NodeId};
use crate::metrics::{Add1, Metrics};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

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
    send_sock: Arc<UdpSocket>,
    tx: Arc<TxTable>,
    token: Arc<Mutex<TokenGenerator>>,
    table: Arc<Mutex<RoutingTable>>,
    harvest: Arc<Mutex<Harvester>>,
    metrics: Arc<Metrics>,
}

impl Router {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        self_id: NodeId,
        self_addr: SocketAddr,
        sybils: Vec<(NodeId, SybilPool)>,
        send_sock: Arc<UdpSocket>,
        tx: Arc<TxTable>,
        token: Arc<Mutex<TokenGenerator>>,
        table: Arc<Mutex<RoutingTable>>,
        harvest: Arc<Mutex<Harvester>>,
        metrics: Arc<Metrics>,
    ) -> Arc<Self> {
        Arc::new(Router {
            self_id,
            self_addr,
            sybils,
            send_sock,
            tx,
            token,
            table,
            harvest,
            metrics,
        })
    }

    pub fn handle_datagram(&self, buf: &[u8], from: SocketAddr) {
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
            Kind::Response { .. } | Kind::Error { .. } => self.handle_reply(msg),
        }
    }

    fn handle_query(&self, t: &Bytes, q: &Bytes, a: &BValue, from: SocketAddr) {
        if let Some(id) = extract_id20(a, b"id") {
            self.insert_nodes(vec![NodeInfo { id, addr: from }]);
        }
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
        let mut sybils: Vec<NodeInfo> = self
            .sybils
            .iter()
            .map(|(id, _)| NodeInfo {
                id: *id,
                addr: self.self_addr,
            })
            .collect();
        sybils.sort_by_key(|n| xor(target, &n.id));
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
        self.do_harvest(ih, crate::harvest::Source::GetPeers);
        let token = self
            .token
            .lock()
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
            .map(|tok| self.token.lock().expect("token").verify(from.ip(), tok))
            .unwrap_or(false);
        if let Some(ih) = extract_id20(a, b"info_hash") {
            match self.classify_pool(&ih) {
                SybilPool::Bep42 => self.metrics.inbound_announce_bep42.add(1),
                SybilPool::Random => self.metrics.inbound_announce_random.add(1),
            }
            if valid {
                self.do_harvest(ih, crate::harvest::Source::AnnouncePeer);
            } else {
                self.metrics.inbound_announce_invalid_token.add(1);
            }
        }
        let r = self.id_response();
        self.send_response(t, from, r);
    }

    fn do_harvest(&self, ih: Infohash, source: crate::harvest::Source) {
        self.metrics.infohashes_harvested.add(1);
        self.harvest
            .lock()
            .expect("harvester poisoned")
            .harvest(ih, source);
    }

    fn handle_reply(&self, msg: Message) {
        if let Some(entry) = self.tx.take(&msg.t)
            && let Some(reply) = entry.reply
        {
            let _ = reply.send(msg);
        }
    }

    fn send_response(&self, t: &Bytes, from: SocketAddr, r: BValue) {
        let msg = Message::response(t.clone(), r);
        let enc = msg.encode();
        self.try_send(&enc, from);
    }

    fn try_send(&self, buf: &[u8], addr: SocketAddr) {
        match self.send_sock.try_send_to(buf, addr) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.metrics.send_dropped.add(1);
            }
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
        if let Err(e) = self.send_sock.send_to(&enc, addr).await {
            self.tx.take(&txid);
            return Err(QueryError::Io(e));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(m)) => Ok(m),
            Ok(Err(_)) => Err(QueryError::Cancelled),
            Err(_) => {
                self.tx.take(&txid);
                self.metrics.outbound_timeouts.add(1);
                Err(QueryError::Timeout)
            }
        }
    }

    fn register(&self, method: &[u8]) -> (Bytes, oneshot::Receiver<Message>) {
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
        for n in nodes {
            table.insert(n);
        }
        self.metrics.routing_table_len.store(table.len() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn routing_nodes(&self) -> Vec<NodeInfo> {
        self.table.lock().expect("routing table poisoned").all()
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
        self.metrics.tx_table_len.add(self.tx.len() as u64);
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
