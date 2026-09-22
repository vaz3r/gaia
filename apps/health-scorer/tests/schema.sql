-- Test schema: minimal production-compatible tables for integration testing.
-- This file creates ONLY the tables needed for health scorer tests.
-- It is NOT a production migration — it's a test fixture.

-- ============================================================
-- torrents table (subset of production columns needed for scorer)
-- ============================================================
CREATE TABLE IF NOT EXISTS torrents (
    infohash                BYTEA PRIMARY KEY,
    name                    TEXT,
    verified_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    first_seen              TIMESTAMPTZ DEFAULT now(),
    last_seen               TIMESTAMPTZ DEFAULT now(),
    total_seen              BIGINT DEFAULT 1,
    total_size              BIGINT DEFAULT 0,
    health_score            SMALLINT DEFAULT 0
                            CHECK (health_score BETWEEN 0 AND 100),
    swarm_peers             INTEGER DEFAULT 0,
    seed_confirmed          BOOLEAN DEFAULT false,
    last_health_check       TIMESTAMPTZ,
    category                VARCHAR(32),
    policy_action           VARCHAR(16)
                            CHECK (policy_action IS NULL OR policy_action IN
                                   ('ALLOW', 'DOWNRANK', 'REVIEW', 'SUPPRESS')),
    fetch_attempts          INTEGER NOT NULL DEFAULT 0
);

-- ============================================================
-- fetch_peer_outcomes (for legacy adapter)
-- ============================================================
CREATE TABLE IF NOT EXISTS fetch_peer_outcomes (
    id          BIGSERIAL PRIMARY KEY,
    infohash    BYTEA NOT NULL,
    result      VARCHAR(32) NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_fpo_infohash_created
    ON fetch_peer_outcomes (infohash, created_at DESC);

-- ============================================================
-- torrent_availability_observations (pre-migration schema)
-- ============================================================
CREATE TABLE IF NOT EXISTS torrent_availability_observations (
    id              BIGSERIAL PRIMARY KEY,
    infohash        BYTEA NOT NULL,
    confirmed_seeds INTEGER DEFAULT 0,
    active_peers    INTEGER DEFAULT 0,
    latency_ms      INTEGER,
    probe_outcome   VARCHAR(32),
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
