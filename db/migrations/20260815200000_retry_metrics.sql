-- 0004_retry_metrics.sql — extend crawl_stats_history with retry-worker metrics
-- and the two new transient infrastructure failure classes.

ALTER TABLE crawl_stats_history
    ADD COLUMN IF NOT EXISTS dht_lookup_failed   BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS lookup_pool_exhausted BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS verified_retried    BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS retry_worker_scans  BIGINT NOT NULL DEFAULT 0;
