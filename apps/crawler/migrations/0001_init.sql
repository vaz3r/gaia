CREATE TABLE IF NOT EXISTS infohash_sightings (
    infohash      BYTEA PRIMARY KEY,
    first_seen    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen     TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_counts JSONB NOT NULL DEFAULT '{}'::jsonb,
    total_seen    BIGINT NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS torrents (
    infohash       BYTEA PRIMARY KEY,
    name           TEXT,
    piece_length   BIGINT,
    pieces         BYTEA,
    total_size     BIGINT,
    file_count     INTEGER,
    files          JSONB,
    fetch_attempts INTEGER NOT NULL DEFAULT 0,
    verified_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT valid_pieces_length CHECK (pieces IS NULL OR octet_length(pieces) % 20 = 0)
);

CREATE INDEX IF NOT EXISTS idx_torrents_verified_at ON torrents (verified_at DESC);
CREATE INDEX IF NOT EXISTS idx_torrents_total_size ON torrents (total_size);

CREATE TABLE IF NOT EXISTS verification_jobs (
    infohash     BYTEA PRIMARY KEY,
    status       TEXT NOT NULL DEFAULT 'pending',
    retry_count  INTEGER NOT NULL DEFAULT 0,
    next_retry_at TIMESTAMPTZ,
    last_error   TEXT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT valid_status CHECK (status IN ('pending', 'verifying', 'verified', 'failed'))
);

CREATE INDEX IF NOT EXISTS idx_verification_jobs_due
    ON verification_jobs (status, next_retry_at)
    WHERE status IN ('pending', 'failed');

CREATE TABLE IF NOT EXISTS metrics (
    ts           TIMESTAMPTZ NOT NULL DEFAULT now(),
    metric_name  TEXT NOT NULL,
    metric_value BIGINT NOT NULL,
    PRIMARY KEY (ts, metric_name)
);

CREATE INDEX IF NOT EXISTS idx_metrics_ts ON metrics (ts DESC);
