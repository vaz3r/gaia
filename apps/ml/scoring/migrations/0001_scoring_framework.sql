-- Migration: 0001_scoring_framework.sql
-- Description: Non-destructive scoring summary columns, score history audit, and availability observation tables.
-- Uses CONCURRENTLY for indexes to guarantee zero disruption to production writes.

-- 1. Summary columns on torrents table (Instantaneous in Postgres 11+)
ALTER TABLE torrents
    ADD COLUMN IF NOT EXISTS integrity_score SMALLINT CHECK (integrity_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS model_safe_probability REAL CHECK (model_safe_probability BETWEEN 0.0 AND 1.0),
    ADD COLUMN IF NOT EXISTS policy_integrity_score SMALLINT CHECK (policy_integrity_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS policy_action VARCHAR(16) CHECK (policy_action IN ('ALLOW', 'DOWNRANK', 'REVIEW', 'SUPPRESS')),
    ADD COLUMN IF NOT EXISTS policy_version VARCHAR(64),
    ADD COLUMN IF NOT EXISTS decision_source VARCHAR(16) CHECK (decision_source IN ('MODEL', 'POLICY', 'MODEL_AND_POLICY', 'MANUAL')),
    ADD COLUMN IF NOT EXISTS metadata_quality_score SMALLINT CHECK (metadata_quality_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS availability_score SMALLINT CHECK (availability_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS risk_tier VARCHAR(16) CHECK (risk_tier IN ('SAFE', 'REVIEW', 'SUSPICIOUS', 'BLOCKED')),
    ADD COLUMN IF NOT EXISTS availability_state VARCHAR(16) CHECK (availability_state IN ('ACTIVE', 'DEGRADED', 'UNKNOWN', 'STALE')),
    ADD COLUMN IF NOT EXISTS score_model_version VARCHAR(64),
    ADD COLUMN IF NOT EXISTS scored_at TIMESTAMPTZ;

-- 2. Immutable Score History & Audit Log
CREATE TABLE IF NOT EXISTS torrent_score_history (
    id                     BIGSERIAL PRIMARY KEY,
    infohash               BYTEA NOT NULL,
    scoring_run_id         UUID NOT NULL,
    model_name             VARCHAR(64) NOT NULL,
    model_version          VARCHAR(64) NOT NULL,
    model_safe_probability REAL NOT NULL,
    policy_integrity_score SMALLINT NOT NULL,
    integrity_score        SMALLINT NOT NULL,
    metadata_quality_score SMALLINT NOT NULL,
    availability_score     SMALLINT NOT NULL,
    risk_tier              VARCHAR(16) NOT NULL,
    policy_action          VARCHAR(16) NOT NULL,
    decision_source        VARCHAR(16) NOT NULL,
    reason_codes           JSONB NOT NULL DEFAULT '[]'::jsonb,
    score_status           VARCHAR(16) NOT NULL DEFAULT 'VALID',
    scored_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_history_run UNIQUE (infohash, scoring_run_id, model_name)
);

CREATE INDEX IF NOT EXISTS idx_score_history_infohash ON torrent_score_history (infohash);
CREATE INDEX IF NOT EXISTS idx_score_history_run_id ON torrent_score_history (scoring_run_id);
CREATE INDEX IF NOT EXISTS idx_score_history_scored_at ON torrent_score_history (scored_at DESC);

-- 3. Dedicated Volatile Availability Observations
CREATE TABLE IF NOT EXISTS torrent_availability_observations (
    id                     BIGSERIAL PRIMARY KEY,
    infohash               BYTEA NOT NULL,
    confirmed_seeds        INTEGER NOT NULL DEFAULT 0,
    active_peers           INTEGER NOT NULL DEFAULT 0,
    probe_latency_ms       INTEGER,
    probe_outcome          VARCHAR(32) NOT NULL,
    observed_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_avail_obs_infohash ON torrent_availability_observations (infohash, observed_at DESC);

-- 4. Checkpoints for resumable backfill scanning
CREATE TABLE IF NOT EXISTS scoring_checkpoints (
    id                     SERIAL PRIMARY KEY,
    backfill_id            UUID NOT NULL,
    last_infohash          BYTEA NOT NULL,
    processed_count        BIGINT NOT NULL DEFAULT 0,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
