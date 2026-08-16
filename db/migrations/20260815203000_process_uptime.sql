-- 0005_process_uptime.sql — persist process-start timestamp so the admin API
-- can compute accurate cumulative rates across crawler restarts (a restart
-- resets the in-process counters, so rate windows must be reset too).

ALTER TABLE crawl_stats_history
    ADD COLUMN IF NOT EXISTS process_start_ts BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS crawl_stats_history_ts_idx
    ON crawl_stats_history (ts);