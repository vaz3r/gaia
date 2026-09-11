CREATE TABLE IF NOT EXISTS pending_infohashes (
    infohash    BYTEA PRIMARY KEY,
    source      TEXT NOT NULL DEFAULT 'bep51',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pending_infohashes_created
    ON pending_infohashes (created_at ASC);
