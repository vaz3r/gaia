-- Migration: 0002_health_scores_create_table.sql
-- Description: Create the canonical health_scores table.
-- Safety: CREATE TABLE IF NOT EXISTS — safe to re-run.
-- Applied via: psql (transactional)

-- ============================================================
-- 1. health_scores table (canonical scoring output)
-- ============================================================

CREATE TABLE IF NOT EXISTS health_scores (
    infohash              VARCHAR(40) PRIMARY KEY,
    health_score          SMALLINT,
    health_state          VARCHAR(16) NOT NULL DEFAULT 'UNKNOWN',
    confidence            REAL NOT NULL DEFAULT 0.0,
    evidence_summary      JSONB,
    algorithm_version     VARCHAR(16) NOT NULL DEFAULT '2.0.0',
    health_calculated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Check constraints
ALTER TABLE health_scores
    ADD CONSTRAINT hs_health_score_range
    CHECK (health_score IS NULL OR (health_score >= 0 AND health_score <= 100))
    NOT VALID;

ALTER TABLE health_scores
    ADD CONSTRAINT hs_confidence_range
    CHECK (confidence >= 0.0 AND confidence <= 1.0)
    NOT VALID;

ALTER TABLE health_scores
    ADD CONSTRAINT hs_health_state_valid
    CHECK (health_state IN ('UNKNOWN', 'UNVERIFIED', 'VERIFIED', 'STALE'))
    NOT VALID;

ALTER TABLE health_scores
    VALIDATE CONSTRAINT hs_health_score_range;

ALTER TABLE health_scores
    VALIDATE CONSTRAINT hs_confidence_range;

ALTER TABLE health_scores
    VALIDATE CONSTRAINT hs_health_state_valid;
