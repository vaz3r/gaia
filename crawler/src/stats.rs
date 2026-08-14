use std::sync::atomic::AtomicU64;

/// Shared crawl counters surfaced by the periodic stats logger.
#[derive(Debug, Default)]
pub struct CrawlStats {
    pub hashes_sampled: AtomicU64,
    pub hashes_unique: AtomicU64,
    /// Fetch requests emitted by the passive announce-intake path.
    pub hashes_announced: AtomicU64,
    /// Passive-intake funnel (crawler-conversion Phase 2).
    pub announces_deduped_redis: AtomicU64,
    pub announces_emitted: AtomicU64,
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
    /// Of the count-1 near-misses: how many had the sole source refresh
    /// (sightings > 1 = plain sparsity) vs a single never-refreshed report
    /// (sightings == 1 = consistent with a backoff-stalled source).
    pub shadow_near_miss_1_sparse: AtomicU64,
    pub shadow_near_miss_1_stalled: AtomicU64,
    /// Number of liveness sweep task invocations (confirms the backstop runs).
    pub liveness_sweeps: AtomicU64,
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
    /// Granular failure taxonomy (crawler-conversion Phase 1).
    pub connection_reset: AtomicU64,
    pub connection_closed: AtomicU64,
    pub parse_error: AtomicU64,
    /// Verified torrents split by discovery source, so we can see which path
    /// actually converts (announce-with-peer-hint vs sampled).
    pub verified_announced: AtomicU64,
    pub verified_sampled: AtomicU64,
    /// Hashes the sparse/stalled discriminator withheld (single-source, never
    /// refreshed). Rising with ~constant verified = the gate is working.
    pub discriminator_filtered: AtomicU64,
    /// get_peers passive-intake funnel: unique hashes emitted / deduped.
    pub lookups_emitted: AtomicU64,
    pub lookups_deduped_redis: AtomicU64,
    /// Verified torrents that entered via the get_peers (sought) path.
    pub verified_lookedup: AtomicU64,
    /// Tracker peer resolution (crawler-conversion): how many fetches got
    /// peers from trackers, and how many verified via a tracker-resolved peer.
    pub tracker_resolved: AtomicU64,
    pub verified_tracker: AtomicU64,
    /// BEP 33 scrape shadow: correlation between seed-bloom presence and
    /// verification (scrape experiment).
    pub scrape_saw_seeds: AtomicU64,
    pub verified_with_seeds: AtomicU64,
    pub verified_without_seeds: AtomicU64,
    pub failed_with_seeds: AtomicU64,
    pub failed_without_seeds: AtomicU64,
}
