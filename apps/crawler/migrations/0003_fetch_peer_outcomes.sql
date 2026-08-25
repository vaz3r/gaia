CREATE TABLE IF NOT EXISTS fetch_peer_outcomes (
    infohash   BYTEA,
    peer       TEXT,
    source     TEXT,
    transport  TEXT,
    result     TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    client     TEXT
);

CREATE INDEX IF NOT EXISTS idx_fpo_created ON fetch_peer_outcomes (created_at);
CREATE INDEX IF NOT EXISTS idx_fpo_result ON fetch_peer_outcomes (result);
CREATE INDEX IF NOT EXISTS idx_fpo_source ON fetch_peer_outcomes (source);
