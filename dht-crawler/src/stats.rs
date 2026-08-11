use std::sync::atomic::AtomicU64;

/// Shared crawl counters surfaced by the periodic stats logger.
#[derive(Debug, Default)]
pub struct CrawlStats {
    pub hashes_sampled: AtomicU64,
    pub hashes_unique: AtomicU64,
    pub fetches_attempted: AtomicU64,
    pub fetches_failed: AtomicU64,
    pub metadata_verified: AtomicU64,
    pub records_persisted: AtomicU64,

    // Per-peer failure breakdown (diagnostic).
    pub connect_timeout: AtomicU64,
    pub connect_refused: AtomicU64,
    pub no_bep10: AtomicU64,
    pub no_ut_metadata: AtomicU64,
    pub metadata_rejected: AtomicU64,
    pub sha1_mismatch: AtomicU64,
    pub empty_peers: AtomicU64,
    pub fetch_deadline: AtomicU64,
    pub peer_errors_other: AtomicU64,
}
