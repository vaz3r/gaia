pub mod bloom;

use crate::harvest::bloom::BloomFilter;
use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use std::net::SocketAddr;
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
    announce_seen: BloomFilter,
    rotate_at: usize,
    discovery_tx: mpsc::Sender<(Infohash, Source)>,
    fresh_verify_tx: mpsc::Sender<Infohash>,
    verify_tx: mpsc::Sender<Infohash>,
    announce_tx: mpsc::Sender<(Infohash, SocketAddr)>,
    metrics: Arc<Metrics>,
}

pub struct HarvestEvent {
    pub ih: Infohash,
    pub source: Source,
    pub direct: Option<SocketAddr>,
}

pub async fn run_harvester(mut rx: mpsc::Receiver<HarvestEvent>, mut harvester: Harvester) {
    while let Some(ev) = rx.recv().await {
        harvester.harvest(ev.ih, ev.source, ev.direct);
    }
}

impl Harvester {
    pub fn new(
        capacity: usize,
        fp_rate: f64,
        announce_bloom_ratio: f64,
        announce_bloom_min: usize,
        discovery_tx: mpsc::Sender<(Infohash, Source)>,
        fresh_verify_tx: mpsc::Sender<Infohash>,
        verify_tx: mpsc::Sender<Infohash>,
        announce_tx: mpsc::Sender<(Infohash, SocketAddr)>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let capacity = capacity.max(64);
        let announce_cap = ((capacity as f64 * announce_bloom_ratio) as usize).max(announce_bloom_min.max(1));
        Harvester {
            current: BloomFilter::new(capacity, fp_rate),
            previous: BloomFilter::new(capacity, fp_rate),
            announce_seen: BloomFilter::new(announce_cap, fp_rate),
            rotate_at: capacity,
            discovery_tx,
            fresh_verify_tx,
            verify_tx,
            announce_tx,
            metrics,
        }
    }

    pub fn harvest(&mut self, ih: Infohash, source: Source, direct: Option<SocketAddr>) -> bool {
        if source == Source::AnnouncePeer && let Some(peer) = direct {
            // Announce sightings get a dedicated bloom so a prior get_peers
            // first-sighting does not suppress the high-value direct fetch.
            if self.announce_seen.contains(&ih) {
                return false;
            }
            self.announce_seen.insert(&ih);
            if self.announce_tx.try_send((ih, peer)).is_err() {
                if self.fresh_verify_tx.try_send(ih).is_err() {
                    self.metrics.fresh_channel_dropped.add(1);
                    return false;
                }
            }
            self.current.insert(&ih);
            if self.current.inserted() >= self.rotate_at {
                std::mem::swap(&mut self.current, &mut self.previous);
                self.current.clear();
            }
            self.metrics.unique_infohashes.add(1);
            crate::trace_lifecycle!(&ih, "discovered", stream = "dht", source = source.tag());
            return true;
        }

        if self.current.contains(&ih) || self.previous.contains(&ih) {
            return false;
        }
        if self.fresh_verify_tx.try_send(ih).is_err() {
            self.metrics.fresh_channel_dropped.add(1);
            return false;
        }
        if self.discovery_tx.try_send((ih, source)).is_err() {
            self.metrics.harvest_sighting_tx_dropped.add(1);
        }
        self.current.insert(&ih);
        if self.current.inserted() >= self.rotate_at {
            std::mem::swap(&mut self.current, &mut self.previous);
            self.current.clear();
        }
        self.metrics.unique_infohashes.add(1);
        crate::trace_lifecycle!(&ih, "discovered", stream = "dht", source = source.tag());
        true
    }

    #[allow(dead_code)]
    pub fn seen(&self) -> usize {
        self.current.inserted() + self.previous.inserted()
    }
}
