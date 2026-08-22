pub mod bloom;

use crate::harvest::bloom::BloomFilter;
use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    GetPeers,
    AnnouncePeer,
}

impl Source {
    pub fn tag(&self) -> &'static str {
        match self {
            Source::GetPeers => "get_peers",
            Source::AnnouncePeer => "announce_peer",
        }
    }
}

pub struct Harvester {
    current: BloomFilter,
    previous: BloomFilter,
    rotate_at: usize,
    discovery_tx: mpsc::Sender<(Infohash, Source)>,
    verify_tx: mpsc::Sender<Infohash>,
    metrics: Arc<Metrics>,
}

impl Harvester {
    pub fn new(
        capacity: usize,
        discovery_tx: mpsc::Sender<(Infohash, Source)>,
        verify_tx: mpsc::Sender<Infohash>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Harvester {
            current: BloomFilter::new(capacity, 0.001),
            previous: BloomFilter::new(capacity, 0.001),
            rotate_at: capacity,
            discovery_tx,
            verify_tx,
            metrics,
        }
    }

    pub fn harvest(&mut self, ih: Infohash, source: Source) -> bool {
        if self.current.contains(&ih) || self.previous.contains(&ih) {
            return false;
        }
        if self.verify_tx.try_send(ih).is_err() {
            return false;
        }
        let _ = self.discovery_tx.try_send((ih, source));
        self.current.insert(&ih);
        if self.current.inserted() >= self.rotate_at {
            std::mem::swap(&mut self.current, &mut self.previous);
            self.current.clear();
        }
        self.metrics.unique_infohashes.add(1);
        crate::trace_lifecycle!(&ih, "discovered", source = source.tag());
        true
    }

    #[allow(dead_code)]
    pub fn seen(&self) -> usize {
        self.current.inserted() + self.previous.inserted()
    }
}
