use std::sync::atomic::AtomicU64;

/// Shared crawl counters surfaced by the periodic stats logger.
#[derive(Debug, Default)]
pub struct CrawlStats {
    pub hashes_sampled: AtomicU64,
    pub hashes_unique: AtomicU64,
    pub fetches_attempted: AtomicU64,
    pub fetches_failed: AtomicU64,
    pub metadata_verified: AtomicU64,
    pub filtered_skip: AtomicU64,
    pub records_persisted: AtomicU64,
}
