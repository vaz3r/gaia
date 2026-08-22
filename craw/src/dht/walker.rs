use crate::dht::routing_table::{NodeInfo, decode_compact};
use crate::krpc::codec::BValue;
use crate::krpc::message::{FIND_NODE, Kind};
use crate::net::rate_limit::RateLimiter;
use crate::router::Router;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Walker {
    router: Arc<Router>,
    limiter: Arc<RateLimiter>,
    bootstrap: Vec<SocketAddr>,
    alpha: usize,
    interval: Duration,
}

impl Walker {
    pub fn new(
        router: Arc<Router>,
        limiter: Arc<RateLimiter>,
        bootstrap: Vec<SocketAddr>,
        alpha: usize,
        interval: Duration,
    ) -> Self {
        Walker {
            router,
            limiter,
            bootstrap,
            alpha,
            interval,
        }
    }

    pub async fn bootstrap(&self, nodes: &[SocketAddr]) {
        let mut set = tokio::task::JoinSet::new();
        for &addr in nodes {
            let router = self.router.clone();
            let target = crate::dht::node_id::random_node_id();
            set.spawn(async move {
                let args = find_node_args(&router.self_id, target);
                router
                    .send_query(FIND_NODE, addr, args, QUERY_TIMEOUT)
                    .await
                    .map(|msg| (msg, addr))
            });
        }
        while let Some(res) = set.join_next().await {
            if let Ok(Ok((msg, addr))) = res {
                ingest(&self.router, &msg, addr);
            }
        }
    }

    pub async fn run(&self) {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            self.step().await;
        }
    }

    async fn step(&self) {
        let nodes = self.router.random_routing_nodes(self.alpha);
        if nodes.is_empty() {
            tracing::warn!("routing table empty, re-bootstrapping");
            self.bootstrap(&self.bootstrap).await;
            return;
        }
        let (our_id, target) = self.pick_target();
        let mut tasks = Vec::new();
        for node in nodes {
            if !self.limiter.allow(node.addr.ip()) {
                continue;
            }
            let router = self.router.clone();
            let args = self.find_node_args(&our_id, target);
            let addr = node.addr;
            tasks.push(tokio::spawn(async move {
                let res = router
                    .send_query(FIND_NODE, addr, args, QUERY_TIMEOUT)
                    .await;
                (res, addr)
            }));
        }
        for task in tasks {
            if let Ok((Ok(msg), addr)) = task.await {
                ingest(&self.router, &msg, addr);
            }
        }
    }

    fn pick_target(&self) -> ([u8; 20], [u8; 20]) {
        let r = rand::random::<f64>();
        if r < 0.10 {
            return (self.router.self_id, self.router.self_id);
        }
        if r < 0.45 {
            let target = crate::dht::node_id::random_node_id();
            return (self.router.self_id, target);
        }
        if self.router.sybils.is_empty() {
            return (
                self.router.self_id,
                crate::dht::node_id::random_node_id(),
            );
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

fn ingest(router: &Router, msg: &crate::krpc::message::Message, addr: SocketAddr) {
    if let Kind::Response { r } = &msg.kind {
        if let Some(id) = extract_id20(r, b"id") {
            router.insert_nodes(vec![NodeInfo { id, addr }]);
        }
        if let Some(nodes) = r.get_bytes(b"nodes") {
            router.insert_nodes(decode_compact(nodes));
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
