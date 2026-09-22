-- Migration: 0001b_health_scoring_indexes.sql
-- Description: CONCURRENTLY indexes for canonical health scoring.
-- IMPORTANT: This file MUST be applied outside a transaction.
--            Use: psql -f this_file.sql  (not inside BEGIN/COMMIT)
--            Or via a migration runner that skips transactions for this file.
--
-- These indexes use CONCURRENTLY to avoid blocking production reads/writes
-- during index creation on large tables.

-- Index for scorer scheduler: find torrents due for recalculation
-- Partial index: only non-SUPPRESSED torrents with a scheduled recalc time
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_recalc_due
    ON torrents (next_health_recalculation_at ASC NULLS LAST)
    WHERE next_health_recalculation_at IS NOT NULL
      AND policy_action IS DISTINCT FROM 'SUPPRESS';

-- Index for calibration snapshots: lookup by infohash
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_calibration_infohash
    ON health_calibration_snapshots (infohash, captured_at DESC);

-- Index for observation retention janitor
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_avail_obs_retention
    ON torrent_availability_observations (observed_at ASC)
    WHERE observation_type IS NOT NULL;
