/** Shared API contract types — kept small and pure so both API and dashboard can mirror them. */

export interface CrawlSnapshotRow {
  ts: string;
  hashes_sampled: number;
  hashes_unique: number;
  hashes_announced: number;
  announces_deduped_redis: number;
  announces_emitted: number;
  shadow_emitted: number;
  shadow_filtered: number;
  shadow_near_miss_1: number;
  shadow_near_miss_2: number;
  shadow_near_miss_1_sparse: number;
  shadow_near_miss_1_stalled: number;
  liveness_sweeps: number;
  fetches_attempted: number;
  fetches_failed: number;
  metadata_verified: number;
  records_persisted: number;
  terminal_dead: number;
  fetch_in_flight: number;
  queue_depth: number;
  connect_timeout: number;
  connect_refused: number;
  connection_reset: number;
  connection_closed: number;
  no_bep10: number;
  no_ut_metadata: number;
  metadata_rejected: number;
  parse_error: number;
  sha1_mismatch: number;
  empty_peers: number;
  fetch_deadline: number;
  early_abort: number;
  peer_errors_other: number;
  dht_lookup_failed: number;
  lookup_pool_exhausted: number;
  verified_announced: number;
  verified_sampled: number;
  verified_retried: number;
  retry_worker_scans: number;
  verified_lookedup: number;
  verified_tracker: number;
  scrape_saw_seeds: number;
  verified_with_seeds: number;
  verified_without_seeds: number;
  failed_with_seeds: number;
  failed_without_seeds: number;
  discriminator_filtered: number;
  lookups_emitted: number;
  lookups_deduped_redis: number;
  routing_nodes: number;
  announced_hashes: number;
  active_lookups: number;
  announce_tokens: number;
  pending_queries: number;
  announces_received: number;
  announces_token_rejected: number;
  announces_suppressed_readonly: number;
  lookups_received: number;
  instance_nodes: unknown;
  unique_per_hr: number | null;
  jemalloc_allocated: number | null;
  jemalloc_active: number | null;
  jemalloc_mapped: number | null;
  jemalloc_retained: number | null;
  net_rx_bytes: number;
  net_tx_bytes: number;
  net_rx_rate_bps: number | null;
  net_tx_rate_bps: number | null;
  host_mem_total: number;
  host_mem_available: number;
  container_mem_current: number;
  cpu_percent: number | null;
  disk_total_bytes: number;
  disk_free_bytes: number;
  loadavg_1: number | null;
  loadavg_5: number | null;
  loadavg_15: number | null;
}

export interface HealthStatus {
  postgres: "healthy" | "unhealthy";
  redis: "healthy" | "unhealthy";
  crawler: "healthy" | "unhealthy" | "unknown";
  api: "healthy";
}

export interface ConfigEntry {
  key: string;
  value: unknown;
  updated_at: string;
}
