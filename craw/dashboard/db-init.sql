-- Dashboard prerequisites. Idempotent; safe to run while the crawler is live.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Fuzzy search + sorting / pagination
CREATE INDEX IF NOT EXISTS idx_torrents_name_trgm    ON torrents USING gin (name gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_torrents_verified_at  ON torrents (verified_at DESC);
CREATE INDEX IF NOT EXISTS idx_torrents_total_size   ON torrents (total_size);
CREATE INDEX IF NOT EXISTS idx_torrents_file_count   ON torrents (file_count);

-- Dashboard runtime lookups
CREATE INDEX IF NOT EXISTS idx_metrics_name_ts       ON metrics (metric_name, ts);
CREATE INDEX IF NOT EXISTS idx_jobs_status           ON verification_jobs (status);
CREATE INDEX IF NOT EXISTS idx_sightings_last_seen   ON infohash_sightings (last_seen);