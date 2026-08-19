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
    (159 - lz).min(159)
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
        if let Some(existing) = bucket.iter_mut().find(|n| n.addr == node.addr) {
            existing.id = node.id;
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
        all.sort_by_key(|node| xor(target, &node.id));
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

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn random_nodes(&self, n: usize) -> Vec<NodeInfo> {
        let mut all = self.all();
        for i in (1..all.len()).rev() {
            let j = rand::random::<u64>() as usize % (i + 1);
            all.swap(i, j);
        }
        all.truncate(n);
        all
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
}
