/** Crawl snapshot row, mirrored from the admin API contract. */
export interface CrawlSnapshot {
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
  verified_announced: number;
  verified_sampled: number;
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

export interface SearchHit {
  info_hash: string;
  name: string;
  size_bytes: number | null;
  file_count: number | null;
  first_seen: number;
  last_seen: number;
  similarity: number | null;
}

export interface SearchResult {
  query: string;
  total: number;
  from: number;
  limit: number;
  data: SearchHit[];
}

export type RangeKey = "5m" | "30m" | "1h" | "6h" | "24h" | "7d";

export type SystemKind = "network" | "memory" | "cpu" | "disk" | "loadavg";

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`GET ${url} -> ${res.status}: ${body.slice(0, 200)}`);
  }
  return (await res.json()) as T;
}

export const api = {
  health: () => getJson<HealthStatus>("/health"),
  latest: () =>
    getJson<CrawlSnapshot | null>("/api/admin/monitor/latest").then((r) =>
      r && "ts" in r ? r : null,
    ),
  history: (metric: string, range: RangeKey) =>
    getJson<{ metric: string; range: string; data: { ts: string; value: number | null }[] }>(
      `/api/admin/monitor/history?metric=${encodeURIComponent(metric)}&range=${range}`,
    ),
  rates: (metric: string, range: RangeKey) =>
    getJson<{ metric: string; range: string; data: { ts: string; value: number | null }[] }>(
      `/api/admin/monitor/rates?metric=${encodeURIComponent(metric)}&range=${range}`,
    ),
  failures: (range: RangeKey) =>
    getJson<{ range: string; data: { reason: string; count: string }[] }>(
      `/api/admin/monitor/failures?range=${range}`,
    ),
  system: (kind: SystemKind, range: RangeKey) =>
    getJson<{ kind: string; range: string; data: Record<string, unknown>[] }>(
      `/api/admin/monitor/system?kind=${kind}&range=${range}`,
    ),
  config: () => getJson<{ data: ConfigEntry[] }>("/api/admin/config"),
  search: (params: SearchParams) => {
    const qs = new URLSearchParams({ q: params.q, limit: String(params.limit) });
    if (params.sort) qs.set("sort", params.sort);
    if (params.order) qs.set("order", params.order);
    if (params.from !== undefined) qs.set("from", String(params.from));
    if (params.sizeMin !== undefined) qs.set("size_min", String(params.sizeMin));
    if (params.sizeMax !== undefined) qs.set("size_max", String(params.sizeMax));
    return getJson<SearchResult>(`/api/search?${qs.toString()}`);
  },
};

export interface SearchParams {
  q: string;
  limit: number;
  sort: "relevance" | "newest" | "largest" | "name";
  order: "asc" | "desc";
  from: number;
  sizeMin?: number | undefined;
  sizeMax?: number | undefined;
}
