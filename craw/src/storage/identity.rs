use crate::dht::node_id::{SybilPool, bep42_node_id_rng, random_node_id};
use crate::krpc::NodeId;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    self_id: [u8; 20],
    external_ip: Option<String>,
    sybils: Vec<SybilEntry>,
}

#[derive(Serialize, Deserialize)]
struct SybilEntry {
    id: [u8; 20],
    pool: SybilPool,
}

pub struct IdentityStore {
    pub self_id: NodeId,
    pub sybils: Vec<(NodeId, SybilPool)>,
}

impl IdentityStore {
    pub fn load_or_create(path: &Path, external_ip: Option<IpAddr>, count: usize) -> Self {
        let ip_key = external_ip.map(|i| i.to_string());
        if let Ok(data) = std::fs::read(path)
            && let Ok(f) = serde_json::from_slice::<IdentityFile>(&data)
        {
            if f.external_ip == ip_key {
                let sybils = f.sybils.iter().map(|e| (e.id, e.pool)).collect();
                return IdentityStore {
                    self_id: f.self_id,
                    sybils,
                };
            }
            let self_id = bep42_or_random(external_ip);
            let sybils = f
                .sybils
                .iter()
                .map(|e| match e.pool {
                    SybilPool::Bep42 => (bep42_or_random(external_ip), SybilPool::Bep42),
                    SybilPool::Random => (e.id, SybilPool::Random),
                })
                .collect();
            return IdentityStore { self_id, sybils };
        }

        let self_id = bep42_or_random(external_ip);
        let sybils = build_fresh(external_ip, count);
        let f = IdentityFile {
            self_id,
            external_ip: ip_key,
            sybils: sybils
                .iter()
                .map(|(id, pool)| SybilEntry {
                    id: *id,
                    pool: *pool,
                })
                .collect(),
        };
        let _ = std::fs::write(path, serde_json::to_vec(&f).unwrap_or_default());
        IdentityStore { self_id, sybils }
    }
}

fn bep42_or_random(external_ip: Option<IpAddr>) -> NodeId {
    match external_ip {
        Some(ip) => bep42_node_id_rng(ip),
        None => random_node_id(),
    }
}

fn build_fresh(external_ip: Option<IpAddr>, n: usize) -> Vec<(NodeId, SybilPool)> {
    let mut ids = Vec::with_capacity(n);
    match external_ip {
        Some(ip) => {
            let bep42 = n / 3;
            for _ in 0..bep42 {
                ids.push((bep42_node_id_rng(ip), SybilPool::Bep42));
            }
            for _ in bep42..n {
                ids.push((random_node_id(), SybilPool::Random));
            }
        }
        None => {
            for _ in 0..n {
                ids.push((random_node_id(), SybilPool::Random));
            }
        }
    }
    ids
}
