use crate::dht::routing_table::{NodeInfo, decode_compact};
use crate::krpc::codec::BValue;
use crate::krpc::message::Kind;
use crate::metrics::Add1;
use crate::net::rate_limit::RateLimiter;
use crate::router::Router;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

pub struct Walker {
    router: Arc<Router>,
    limiter: Arc<RateLimiter>,
    bootstrap: Vec<SocketAddr>,
    alpha: usize,
    interval: Duration,
    query_timeout: Duration,
    self_explore_prob: f64,
    parse_nodes6: bool,
}

impl Walker {
    pub fn new(
        router: Arc<Router>,
        limiter: Arc<RateLimiter>,
        bootstrap: Vec<SocketAddr>,
        alpha: usize,
        interval: Duration,
        query_timeout: Duration,
        self_explore_prob: f64,
        parse_nodes6: bool,
    ) -> Self {
        Walker {
            router,
            limiter,
            bootstrap,
            alpha,
            interval,
            query_timeout,
            self_explore_prob,
            parse_nodes6,
        }
    }

    pub async fn bootstrap(&self, nodes: &[SocketAddr]) {
        eprintln!("[DBG] bootstrap: starting with {} nodes", nodes.len());
        let mut set = tokio::task::JoinSet::new();
        let query_timeout = self.query_timeout;
        for &addr in nodes {
            let router = self.router.clone();
            let target = crate::dht::node_id::random_node_id();
            let sender = self.router.random_sybil_id();
            set.spawn(async move {
                router
                    .send_find_node_fast(addr, &target, &sender, query_timeout)
                    .await
            });
        }
        while let Some(res) = set.join_next().await {
            if let Ok(Ok((nodes, nodes6))) = res {
                self.router.metrics().walker_ok.add(1);
                self.router
                    .metrics()
                    .walker_nodes_returned
                    .add(nodes.len() as u64);
                self.router.insert_nodes(nodes);
                if self.parse_nodes6 {
                    self.router
                        .metrics()
                        .walker_nodes_returned
                        .add(nodes6.len() as u64);
                    self.router.insert_nodes(nodes6);
                }
            }
        }
        eprintln!("[DBG] bootstrap: done");
    }

    pub async fn run(&self) {
        eprintln!("[DBG] walker.run: starting");
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut set = tokio::task::JoinSet::new();
        loop {
            interval.tick().await;
            self.reap(&mut set).await;
            if !self.spawn_step(&mut set) {
                tracing::warn!("routing table empty, re-bootstrapping");
                self.bootstrap(&self.bootstrap).await;
                continue;
            }
        }
    }

    fn handle(
        &self,
        res: Result<
            (
                crate::krpc::NodeId,
                Result<(Vec<NodeInfo>, Vec<NodeInfo>), crate::router::QueryError>,
            ),
            tokio::task::JoinError,
        >,
    ) {
        if let Ok((node_id, q_res)) = res {
            match q_res {
                Ok((nodes, nodes6)) => {
                    self.router.metrics().walker_ok.add(1);
                    self.router
                        .metrics()
                        .walker_nodes_returned
                        .add(nodes.len() as u64);
                    let useful = !nodes.is_empty() || !nodes6.is_empty();
                    self.router.record_query(&node_id, useful, false);
                    self.router.insert_nodes(nodes);
                    if self.parse_nodes6 {
                        self.router
                            .metrics()
                            .walker_nodes_returned
                            .add(nodes6.len() as u64);
                        self.router.insert_nodes(nodes6);
                    }
                }
                Err(_) => {
                    self.router.record_query(&node_id, false, true);
                }
            }
        }
    }

    async fn reap(
        &self,
        set: &mut tokio::task::JoinSet<(
            crate::krpc::NodeId,
            Result<(Vec<NodeInfo>, Vec<NodeInfo>), crate::router::QueryError>,
        )>,
    ) {
        loop {
            match set.try_join_next() {
                Some(res) => self.handle(res),
                None => break,
            }
        }
    }

    fn spawn_step(
        &self,
        set: &mut tokio::task::JoinSet<(
            crate::krpc::NodeId,
            Result<(Vec<NodeInfo>, Vec<NodeInfo>), crate::router::QueryError>,
        )>,
    ) -> bool {
        self.router.metrics().walker_steps.add(1);
        let nodes = self.router.random_routing_nodes(self.alpha);
        if nodes.is_empty() {
            return false;
        }
        let (our_id, target) = self.pick_target();
        let query_timeout = self.query_timeout;
        for node in nodes {
            if !self.limiter.allow(node.addr.ip()) {
                continue;
            }
            let router = self.router.clone();
            let addr = node.addr;
            self.router.metrics().walker_queries.add(1);
            let node_id = node.id;
            set.spawn(async move {
                let res = router
                    .send_find_node_fast(addr, &target, &our_id, query_timeout)
                    .await;
                (node_id, res)
            });
        }
        true
    }

    fn pick_target(&self) -> ([u8; 20], [u8; 20]) {
        let explore = rand::random::<f64>() < self.self_explore_prob;
        if explore {
            self.router.metrics().walker_self_target.add(1);
            if let Some(n) = self.router.random_routing_nodes(1).first() {
                return (self.router.random_sybil_id(), n.id);
            }
        }
        self.router.metrics().walker_sybil_target.add(1);
        if self.router.sybils.is_empty() {
            return (self.router.self_id, crate::dht::node_id::random_node_id());
        }
        let idx = rand::random::<u64>() as usize % self.router.sybils.len();
        let (sybil, _) = self.router.sybils[idx];
        (sybil, sybil)
    }

    fn find_node_args(&self, id: &[u8; 20], target: [u8; 20]) -> BValue {
        find_node_args(id, target)
    }
}

fn find_node_args(id: &[u8; 20], target: [u8; 20]) -> BValue {
    BValue::dict(vec![
        (
            Bytes::from_static(b"id"),
            BValue::Bytes(Bytes::copy_from_slice(id)),
        ),
        (
            Bytes::from_static(b"target"),
            BValue::Bytes(Bytes::copy_from_slice(&target)),
        ),
    ])
}

fn ingest(
    router: &Router,
    msg: &crate::krpc::message::Message,
    addr: SocketAddr,
    parse_nodes6: bool,
) {
    if let Kind::Response { r } = &msg.kind {
        router.metrics().walker_ok.add(1);
        if let Some(id) = extract_id20(r, b"id") {
            router.insert_nodes(vec![NodeInfo {
                id,
                addr,
                query_count: 0,
                fail_count: 0,
                last_useful: true,
            }]);
        }
        if let Some(nodes) = r.get_bytes(b"nodes") {
            let decoded = decode_compact(nodes);
            router
                .metrics()
                .walker_nodes_returned
                .add(decoded.len() as u64);
            router.insert_nodes(decoded);
        }
        if parse_nodes6 && let Some(nodes) = r.get_bytes(b"nodes6") {
            let decoded = crate::dht::routing_table::decode_compact6(nodes);
            router
                .metrics()
                .walker_nodes_returned
                .add(decoded.len() as u64);
            router.insert_nodes(decoded);
        }
    }
}

fn extract_id20(v: &BValue, key: &[u8]) -> Option<[u8; 20]> {
    let b = v.get_bytes(key)?;
    if b.len() != 20 {
        return None;
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(b);
    Some(out)
}
