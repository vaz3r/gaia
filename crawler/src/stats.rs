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
    /// Hashes whose failed attempts reached `--max-attempts` and were cached
    /// in the bloom as terminal dead (never re-emitted).
    pub terminal_dead: AtomicU64,

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

/// One immutable monitoring snapshot captured at a stats tick and persisted to
/// `crawl_stats_history`. Counter values are cumulative; rates are derived by
/// the admin API via window functions.
#[derive(Debug, Clone, Default)]
pub struct CrawlSnapshot {
    pub hashes_sampled: u64,
    pub hashes_unique: u64,
    pub hashes_announced: u64,
    pub announces_deduped_redis: u64,
    pub announces_emitted: u64,
    pub shadow_emitted: u64,
    pub shadow_filtered: u64,
    pub shadow_near_miss_1: u64,
    pub shadow_near_miss_2: u64,
    pub shadow_near_miss_1_sparse: u64,
    pub shadow_near_miss_1_stalled: u64,
    pub liveness_sweeps: u64,
    pub fetches_attempted: u64,
    pub fetches_failed: u64,
    pub metadata_verified: u64,
    pub records_persisted: u64,
    pub terminal_dead: u64,
    pub fetch_in_flight: u64,
    pub queue_depth: u64,
    pub connect_timeout: u64,
    pub connect_refused: u64,
    pub connection_reset: u64,
    pub connection_closed: u64,
    pub no_bep10: u64,
    pub no_ut_metadata: u64,
    pub metadata_rejected: u64,
    pub parse_error: u64,
    pub sha1_mismatch: u64,
    pub empty_peers: u64,
    pub fetch_deadline: u64,
    pub early_abort: u64,
    pub peer_errors_other: u64,
    pub verified_announced: u64,
    pub verified_sampled: u64,
    pub verified_lookedup: u64,
    pub verified_tracker: u64,
    pub scrape_saw_seeds: u64,
    pub verified_with_seeds: u64,
    pub verified_without_seeds: u64,
    pub failed_with_seeds: u64,
    pub failed_without_seeds: u64,
    pub discriminator_filtered: u64,
    pub lookups_emitted: u64,
    pub lookups_deduped_redis: u64,
    pub routing_nodes: u64,
    pub announced_hashes: u64,
    pub active_lookups: u64,
    pub announce_tokens: u64,
    pub pending_queries: u64,
    pub announces_received: u64,
    pub announces_token_rejected: u64,
    pub announces_suppressed_readonly: u64,
    pub lookups_received: u64,
    pub unique_per_hr: f64,
    pub jemalloc_allocated: f64,
    pub jemalloc_active: f64,
    pub jemalloc_mapped: f64,
    pub jemalloc_retained: f64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_rx_rate_bps: f64,
    pub net_tx_rate_bps: f64,
    pub host_mem_total: u64,
    pub host_mem_available: u64,
    pub container_mem_current: u64,
    pub cpu_percent: f64,
    pub disk_total_bytes: u64,
    pub disk_free_bytes: u64,
    pub loadavg_1: f64,
    pub loadavg_5: f64,
    pub loadavg_15: f64,
}
