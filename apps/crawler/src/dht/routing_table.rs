use crate::krpc::NodeId;
use std::collections::VecDeque;
use std::net::SocketAddr;

pub const K: usize = 8;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NodeInfo {
    pub id: NodeId,
    pub addr: SocketAddr,
}

pub fn xor(a: &NodeId, b: &NodeId) -> NodeId {
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = a[i] ^ b[i];
    }
    out
}


pub fn cmp_xor(target: &NodeId, a: &NodeId, b: &NodeId) -> std::cmp::Ordering {
    for i in 0..20 {
        let xa = a[i] ^ target[i];
        let xb = b[i] ^ target[i];
        if xa != xb {
            return xa.cmp(&xb);
        }
    }
    std::cmp::Ordering::Equal
}

fn leading_zeros(x: &NodeId) -> usize {
    let mut count = 0;
    for &byte in x.iter() {
        if byte == 0 {
            count += 8;
        } else {
            count += byte.leading_zeros() as usize;
            break;
        }
    }
    count
}

fn bucket_index(self_id: &NodeId, id: &NodeId) -> usize {
    let d = xor(self_id, id);
    let lz = leading_zeros(&d);
    if lz >= 160 {
        0
    } else {
        159 - lz
    }
}

pub struct RoutingTable {
    self_id: NodeId,
    buckets: Vec<VecDeque<NodeInfo>>,
}

impl RoutingTable {
    pub fn new(self_id: NodeId) -> Self {
        RoutingTable {
            self_id,
            buckets: (0..160).map(|_| VecDeque::new()).collect(),
        }
    }

    #[allow(dead_code)]
    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    pub fn insert(&mut self, node: NodeInfo) -> bool {
        if node.id == self.self_id {
            return false;
        }
        let idx = bucket_index(&self.self_id, &node.id);
        let bucket = &mut self.buckets[idx];
        if let Some(existing) = bucket.iter_mut().find(|n| n.id == node.id) {
            existing.addr = node.addr;
            return true;
        }
        if bucket.len() >= K {
            bucket.pop_front();
        }
        bucket.push_back(node);
        true
    }

    pub fn closest(&self, target: &NodeId, n: usize) -> Vec<NodeInfo> {
        let mut all: Vec<NodeInfo> = self
            .buckets
            .iter()
            .flat_map(|b| b.iter())
            .cloned()
            .collect();
        all.sort_unstable_by(|a, b| cmp_xor(target, &a.id, &b.id));
        all.truncate(n);
        all
    }

    pub fn all(&self) -> Vec<NodeInfo> {
        self.buckets
            .iter()
            .flat_map(|b| b.iter())
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.buckets.iter().map(|b| b.len()).sum()
    }

    pub fn buckets_used(&self) -> usize {
        self.buckets.iter().filter(|b| !b.is_empty()).count()
    }

    pub fn contains_id(&self, id: &NodeId) -> bool {
        self.buckets.iter().any(|b| b.iter().any(|n| &n.id == id))
    }

    #[cfg(test)]
    fn bucket_fill(&self, idx: usize) -> usize {
        self.buckets.get(idx).map(|b| b.len()).unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn random_nodes(&self, n: usize) -> Vec<NodeInfo> {
        let non_empty: Vec<usize> = (0..160)
            .filter(|&i| !self.buckets[i].is_empty())
            .collect();
        if non_empty.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(n);
        let mut seen = std::collections::HashSet::new();
        let mut attempts = 0usize;
        let max_attempts = n * 32 + 8;
        while result.len() < n && attempts < max_attempts {
            attempts += 1;
            let bidx = non_empty[rand::random::<u64>() as usize % non_empty.len()];
            let bucket = &self.buckets[bidx];
            let nidx = rand::random::<u64>() as usize % bucket.len();
            let node = bucket[nidx].clone();
            if seen.insert(node.id) {
                result.push(node);
            }
        }
        result
    }
}

pub fn encode_compact(nodes: &[NodeInfo]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nodes.len() * 26);
    for n in nodes {
        if let std::net::IpAddr::V4(v4) = n.addr.ip() {
            out.extend_from_slice(&n.id);
            out.extend_from_slice(&v4.octets());
            out.extend_from_slice(&n.addr.port().to_be_bytes());
        }
    }
    out
}

pub fn decode_compact(bytes: &[u8]) -> Vec<NodeInfo> {
    let mut nodes = Vec::new();
    let mut i = 0;
    while i + 26 <= bytes.len() {
        let mut id = [0u8; 20];
        id.copy_from_slice(&bytes[i..i + 20]);
        let ip =
            std::net::Ipv4Addr::new(bytes[i + 20], bytes[i + 21], bytes[i + 22], bytes[i + 23]);
        let port = u16::from_be_bytes([bytes[i + 24], bytes[i + 25]]);
        nodes.push(NodeInfo {
            id,
            addr: SocketAddr::new(std::net::IpAddr::V4(ip), port),
        });
        i += 26;
    }
    nodes
}

pub fn decode_compact6(bytes: &[u8]) -> Vec<NodeInfo> {
    let mut nodes = Vec::new();
    let mut i = 0;
    while i + 38 <= bytes.len() {
        let mut id = [0u8; 20];
        id.copy_from_slice(&bytes[i..i + 20]);
        let mut v6 = [0u8; 16];
        v6.copy_from_slice(&bytes[i + 20..i + 36]);
        let port = u16::from_be_bytes([bytes[i + 36], bytes[i + 37]]);
        nodes.push(NodeInfo {
            id,
            addr: SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::from(v6)), port),
        });
        i += 38;
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_closest() {
        let mut rt = RoutingTable::new([0u8; 20]);
        let mut nodes = Vec::new();
        for i in 1..17u8 {
            let mut id = [0u8; 20];
            id[0] = i;
            let addr: SocketAddr = format!("1.2.3.{}:{}", i, 6881).parse().unwrap();
            let ni = NodeInfo { id, addr };
            rt.insert(ni.clone());
            nodes.push(ni);
        }
        assert_eq!(rt.len(), 16);
        let mut target = [0u8; 20];
        target[0] = 3;
        let closest = rt.closest(&target, 4);
        assert_eq!(closest.len(), 4);
        assert_eq!(closest[0].id, target);
    }

    #[test]
    fn random_nodes_populated_table() {
        let mut rt = RoutingTable::new([0u8; 20]);
        for i in 0..400u16 {
            let mut id = [0u8; 20];
            id[0] = (i >> 8) as u8;
            id[1] = (i & 0xff) as u8;
            let addr: SocketAddr = format!("10.{}.{}.{}:6881", i % 200, (i / 200) % 200, i % 250)
                .parse()
                .unwrap();
            rt.insert(NodeInfo { id, addr });
        }
        assert!(rt.len() > 0);
        for _ in 0..100 {
            let nodes = rt.random_nodes(3);
            assert!(!nodes.is_empty());
            assert!(nodes.len() <= 3);
        }
    }

    #[test]
    fn flood_of_new_ids_reaches_kademlia_equilibrium() {
        // Kademlia XOR buckets concentrate node ids exponentially in the most
        // distant buckets: bucket 159 holds ~50% of uniform ids, 158 ~25%, ...
        // So a flood of distinct ids saturates only the far buckets (K=8 each)
        // and the total length plateaus well below 160*K. This documents the
        // intrinsic property: a healthy table is ~100-400 nodes, not 1280.
        let mut rt = RoutingTable::new([0u8; 20]);
        let mut new_at_insert = 0u64;
        for i in 0..500_000u64 {
            let id = rand::random::<[u8; 20]>();
            if !rt.contains_id(&id) {
                new_at_insert += 1;
            }
            rt.insert(NodeInfo {
                id,
                addr: format!("1.2.3.{}:6881", (i % 250) as u8 + 1).parse().unwrap(),
            });
        }
        assert_eq!(new_at_insert, 500_000);
        let top = rt.bucket_fill(159) + rt.bucket_fill(158) + rt.bucket_fill(157);
        let total = rt.buckets_used();
        assert!(rt.len() > 50 && rt.len() < 512, "len={}", rt.len());
        assert!(total < 40, "only far buckets fill under uniform ids, used={total}");
        let _ = top;
    }
}
