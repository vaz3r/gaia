use dashmap::DashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub struct PeerCache {
    bad: DashMap<SocketAddr, Instant>,
    ttl: Duration,
    max_entries: usize,
}

impl PeerCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        PeerCache {
            bad: DashMap::with_capacity_and_shard_amount(1024, 64),
            ttl,
            max_entries,
        }
    }

    pub fn mark_bad(&self, addr: SocketAddr) {
        self.bad.insert(addr, Instant::now());
        self.enforce_bound();
    }

    pub fn is_bad(&self, addr: &SocketAddr) -> bool {
        match self.bad.get(addr) {
            Some(entry) => {
                if entry.elapsed() < self.ttl {
                    true
                } else {
                    drop(entry);
                    self.bad.remove(addr);
                    false
                }
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.bad.len()
    }

    pub fn evict_expired(&self) -> usize {
        let now = Instant::now();
        let mut evicted = 0;
        self.bad.retain(|_, expiry| {
            if now.duration_since(*expiry) >= self.ttl {
                evicted += 1;
                false
            } else {
                true
            }
        });
        evicted
    }

    fn enforce_bound(&self) {
        if self.bad.len() <= self.max_entries {
            return;
        }
        let _ = self.evict_expired();
        if self.bad.len() <= self.max_entries {
            return;
        }
        let excess = self.bad.len() - self.max_entries;
        let target_remove = (excess / 8).max(1);
        let mut to_remove = Vec::with_capacity(target_remove);
        for entry in self.bad.iter() {
            if to_remove.len() >= target_remove {
                break;
            }
            to_remove.push(*entry.key());
        }
        for key in to_remove {
            self.bad.remove(&key);
        }
    }
}
