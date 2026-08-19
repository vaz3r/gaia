use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub inbound_ping: AtomicU64,
    pub inbound_find_node: AtomicU64,
    pub inbound_get_peers: AtomicU64,
    pub inbound_announce_peer: AtomicU64,
    pub inbound_invalid: AtomicU64,
    pub inbound_find_node_bep42: AtomicU64,
    pub inbound_find_node_random: AtomicU64,
    pub inbound_get_peers_bep42: AtomicU64,
    pub inbound_get_peers_random: AtomicU64,
    pub inbound_announce_bep42: AtomicU64,
    pub inbound_announce_random: AtomicU64,
    pub inbound_announce_invalid_token: AtomicU64,
    pub tokens_issued: AtomicU64,
    pub infohashes_harvested: AtomicU64,
    pub unique_infohashes: AtomicU64,
    pub outbound_queries: AtomicU64,
    pub outbound_timeouts: AtomicU64,
    pub tx_table_len: AtomicU64,
    pub routing_table_len: AtomicU64,
    pub verify_attempts: AtomicU64,
    pub verify_success: AtomicU64,
    pub verify_fail: AtomicU64,
    pub verify_timeouts: AtomicU64,
    pub fetch_attempts: AtomicU64,
    pub source_queries: AtomicU64,
    pub source_responses: AtomicU64,
    pub source_peers_returned: AtomicU64,
    pub send_dropped: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics::default()
    }
}

pub trait Add1 {
    fn add(&self, n: u64);
}

impl Add1 for AtomicU64 {
    fn add(&self, n: u64) {
        self.fetch_add(n, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot {
    pub inbound_ping: u64,
    pub inbound_find_node: u64,
    pub inbound_get_peers: u64,
    pub inbound_announce_peer: u64,
    pub inbound_invalid: u64,
    pub inbound_find_node_bep42: u64,
    pub inbound_find_node_random: u64,
    pub inbound_get_peers_bep42: u64,
    pub inbound_get_peers_random: u64,
    pub inbound_announce_bep42: u64,
    pub inbound_announce_random: u64,
    pub inbound_announce_invalid_token: u64,
    pub tokens_issued: u64,
    pub infohashes_harvested: u64,
    pub unique_infohashes: u64,
    pub outbound_queries: u64,
    pub outbound_timeouts: u64,
    pub tx_table_len: u64,
    pub routing_table_len: u64,
    pub verify_attempts: u64,
    pub verify_success: u64,
    pub verify_fail: u64,
    pub verify_timeouts: u64,
    pub fetch_attempts: u64,
    pub source_queries: u64,
    pub source_responses: u64,
    pub source_peers_returned: u64,
    pub send_dropped: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            inbound_ping: self.inbound_ping.load(Ordering::Relaxed),
            inbound_find_node: self.inbound_find_node.load(Ordering::Relaxed),
            inbound_get_peers: self.inbound_get_peers.load(Ordering::Relaxed),
            inbound_announce_peer: self.inbound_announce_peer.load(Ordering::Relaxed),
            inbound_invalid: self.inbound_invalid.load(Ordering::Relaxed),
            inbound_find_node_bep42: self.inbound_find_node_bep42.load(Ordering::Relaxed),
            inbound_find_node_random: self.inbound_find_node_random.load(Ordering::Relaxed),
            inbound_get_peers_bep42: self.inbound_get_peers_bep42.load(Ordering::Relaxed),
            inbound_get_peers_random: self.inbound_get_peers_random.load(Ordering::Relaxed),
            inbound_announce_bep42: self.inbound_announce_bep42.load(Ordering::Relaxed),
            inbound_announce_random: self.inbound_announce_random.load(Ordering::Relaxed),
            inbound_announce_invalid_token: self
                .inbound_announce_invalid_token
                .load(Ordering::Relaxed),
            tokens_issued: self.tokens_issued.load(Ordering::Relaxed),
            infohashes_harvested: self.infohashes_harvested.load(Ordering::Relaxed),
            unique_infohashes: self.unique_infohashes.load(Ordering::Relaxed),
            outbound_queries: self.outbound_queries.load(Ordering::Relaxed),
            outbound_timeouts: self.outbound_timeouts.load(Ordering::Relaxed),
            tx_table_len: self.tx_table_len.load(Ordering::Relaxed),
            routing_table_len: self.routing_table_len.load(Ordering::Relaxed),
            verify_attempts: self.verify_attempts.load(Ordering::Relaxed),
            verify_success: self.verify_success.load(Ordering::Relaxed),
            verify_fail: self.verify_fail.load(Ordering::Relaxed),
            verify_timeouts: self.verify_timeouts.load(Ordering::Relaxed),
            fetch_attempts: self.fetch_attempts.load(Ordering::Relaxed),
            source_queries: self.source_queries.load(Ordering::Relaxed),
            source_responses: self.source_responses.load(Ordering::Relaxed),
            source_peers_returned: self.source_peers_returned.load(Ordering::Relaxed),
            send_dropped: self.send_dropped.load(Ordering::Relaxed),
        }
    }
}
