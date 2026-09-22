-- Migration: 0001a_health_scoring_tables.sql
-- Description: Non-destructive schema changes for canonical health scoring.
-- Safety: No DROP TABLE, no data deletion, all constraints NOT VALID first.
-- Applied via: psql (transactional, safe to run inside BEGIN/COMMIT)

-- ============================================================
-- 1. Expand torrent_availability_observations
-- ============================================================
-- Existing table (from apps/ml/scoring/migrations/0001_scoring_framework.sql):
--   id, infohash, confirmed_seeds, active_peers, probe_latency_ms,
--   probe_outcome, observed_at
--
-- New columns added non-destructively. Old columns retained until
-- compatibility is confirmed in a later cleanup migration.

ALTER TABLE torrent_availability_observations
    ADD COLUMN IF NOT EXISTS observation_type VARCHAR(32),
    ADD COLUMN IF NOT EXISTS peer_count INTEGER,
    ADD COLUMN IF NOT EXISTS seed_count INTEGER,
    ADD COLUMN IF NOT EXISTS source VARCHAR(32) NOT NULL DEFAULT 'crawler',
    ADD COLUMN IF NOT EXISTS failure_reason TEXT,
    ADD COLUMN IF NOT EXISTS raw_data JSONB;

-- Backfill observation_type from legacy probe_outcome where possible
UPDATE torrent_availability_observations
SET observation_type = CASE
    WHEN probe_outcome IN ('ok', 'metadata_ok') THEN 'probe_success'
    WHEN probe_outcome IN ('timeout', 'metadata_timeout') THEN 'probe_failure'
    WHEN probe_outcome = 'no_peers' THEN 'probe_failure'
    WHEN probe_outcome = 'sha1_mismatch' THEN 'probe_failure'
    ELSE 'probe_failure'
END
WHERE observation_type IS NULL;

-- Backfill peer_count/seed_count from legacy columns
UPDATE torrent_availability_observations
SET peer_count = active_peers,
    seed_count = confirmed_seeds
WHERE peer_count IS NULL;

-- Now add NOT NULL constraints on backfilled columns
ALTER TABLE torrent_availability_observations
    ALTER COLUMN observation_type SET NOT NULL,
    ALTER COLUMN peer_count SET NOT NULL,
    ALTER COLUMN seed_count SET NOT NULL;

-- Add check constraints (NOT VALID first, then validate after backfill)
ALTER TABLE torrent_availability_observations
    ADD CONSTRAINT observations_type_check
    CHECK (observation_type IN (
        'dht_sighting', 'peer_seen', 'seed_confirmed',
        'metadata_fetch_success', 'metadata_fetch_failure',
        'probe_success', 'probe_failure'
    )) NOT VALID;

ALTER TABLE torrent_availability_observations
    ADD CONSTRAINT observations_peer_count_nonneg
    CHECK (peer_count >= 0) NOT VALID;

ALTER TABLE torrent_availability_observations
    ADD CONSTRAINT observations_seed_count_nonneg
    CHECK (seed_count >= 0) NOT VALID;

ALTER TABLE torrent_availability_observations
    ADD CONSTRAINT observations_latency_nonneg
    CHECK (latency_ms IS NULL OR latency_ms >= 0) NOT VALID;

-- Validate constraints (safe to run, acquires SHARE UPDATE EXCLUSIVE lock briefly)
ALTER TABLE torrent_availability_observations
    VALIDATE CONSTRAINT observations_type_check;

ALTER TABLE torrent_availability_observations
    VALIDATE CONSTRAINT observations_peer_count_nonneg;

ALTER TABLE torrent_availability_observations
    VALIDATE CONSTRAINT observations_seed_count_nonneg;

ALTER TABLE torrent_availability_observations
    VALIDATE CONSTRAINT observations_latency_nonneg;

-- Index for cursor-based scanning (non-concurrent is fine for small tables)
CREATE INDEX IF NOT EXISTS idx_avail_obs_id
    ON torrent_availability_observations (id);

-- ============================================================
-- 2. Canonical health fields on torrents
-- ============================================================

ALTER TABLE torrents
    ADD COLUMN IF NOT EXISTS health_confidence REAL,
    ADD COLUMN IF NOT EXISTS health_calculated_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS health_algorithm_version VARCHAR(16),
    ADD COLUMN IF NOT EXISTS health_state VARCHAR(16),
    ADD COLUMN IF NOT EXISTS last_verified_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_seed_confirmed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS swarm_peers_observed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS next_health_recalculation_at TIMESTAMPTZ;

-- Check constraints (NOT VALID first)
ALTER TABLE torrents
    ADD CONSTRAINT torrents_health_confidence_range
    CHECK (health_confidence IS NULL OR (health_confidence >= 0.0 AND health_confidence <= 1.0))
    NOT VALID;

ALTER TABLE torrents
    ADD CONSTRAINT torrents_health_state_valid
    CHECK (health_state IS NULL OR health_state IN ('UNKNOWN', 'UNVERIFIED', 'VERIFIED', 'STALE'))
    NOT VALID;

-- Validate after deployment
ALTER TABLE torrents
    VALIDATE CONSTRAINT torrents_health_confidence_range;

ALTER TABLE torrents
    VALIDATE CONSTRAINT torrents_health_state_valid;

-- ============================================================
-- 3. Scorer cursor state (durable high-water mark)
-- ============================================================

CREATE TABLE IF NOT EXISTS health_scoring_cursor (
    id                  INTEGER PRIMARY KEY DEFAULT 1,
    last_processed_id   BIGINT NOT NULL DEFAULT 0,
    last_processed_at   TIMESTAMPTZ NOT NULL DEFAULT '2000-01-01T00:00:00Z',
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT single_row CHECK (id = 1)
);

INSERT INTO health_scoring_cursor (id, last_processed_id, last_processed_at)
VALUES (1, 0, '2000-01-01T00:00:00Z')
ON CONFLICT (id) DO NOTHING;

-- ============================================================
-- 4. Calibration snapshots
-- ============================================================

CREATE TABLE IF NOT EXISTS health_calibration_snapshots (
    id                      BIGSERIAL PRIMARY KEY,
    infohash                BYTEA NOT NULL,
    score_before            SMALLINT,
    health_confidence       REAL,
    score_calculated_at     TIMESTAMPTZ,
    verification_started_at TIMESTAMPTZ,
    verification_ended_at   TIMESTAMPTZ,
    probe_type              VARCHAR(32)
        CHECK (probe_type IS NULL OR probe_type IN (
            'metadata_fetch', 'peer_connect', 'dht_probe', 'other'
        )),
    probe_result            BOOLEAN,
    score_age_hours         REAL,
    algorithm_version       VARCHAR(16),
    captured_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
