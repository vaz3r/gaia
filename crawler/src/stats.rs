use std::sync::atomic::AtomicU64;

/// Shared crawl counters surfaced by the periodic stats logger.
#[derive(Debug, Default)]
pub struct CrawlStats {
    pub hashes_sampled: AtomicU64,
    pub hashes_unique: AtomicU64,
    /// Fetch requests emitted by the passive announce-intake path.
    pub hashes_announced: AtomicU64,
    /// Shadow-mode liveness gate: hashes that reached the shadow threshold
    /// (would be emitted under `--min-seen-shadow`).
    pub shadow_emitted: AtomicU64,
    /// Shadow-mode liveness gate: hashes that expired below the shadow
    /// threshold (would be filtered under `--min-seen-shadow`).
    pub shadow_filtered: AtomicU64,
    /// Shadow-mode near-miss buckets: expired having reached exactly 1 or 2
    /// distinct sources (detect window/threshold coupling).
    pub shadow_near_miss_1: AtomicU64,
    pub shadow_near_miss_2: AtomicU64,
    pub fetches_attempted: AtomicU64,
    pub fetches_failed: AtomicU64,
    pub metadata_verified: AtomicU64,
    pub records_persisted: AtomicU64,

    // Pipeline depth (snapshots, not cumulative).
    pub fetch_in_flight: AtomicU64,
    pub queue_depth: AtomicU64,

    // Per-peer failure breakdown (diagnostic).
    pub connect_timeout: AtomicU64,
    pub connect_refused: AtomicU64,
    pub no_bep10: AtomicU64,
    pub no_ut_metadata: AtomicU64,
    pub metadata_rejected: AtomicU64,
    pub sha1_mismatch: AtomicU64,
    pub empty_peers: AtomicU64,
    pub fetch_deadline: AtomicU64,
    pub early_abort: AtomicU64,
    pub peer_errors_other: AtomicU64,
}
