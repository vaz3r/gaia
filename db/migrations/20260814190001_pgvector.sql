-- 0002_pgvector.sql — reserve pgvector + embeddings skeleton for the future
-- classifier/embedder stage. No code consumes this yet; it exists so the
-- semantic-search milestone is purely additive.

CREATE EXTENSION IF NOT EXISTS vector;

-- Reserved: per-torrent embedding (768-dim placeholder) + metadata. The
-- dimension will be fixed when the embedder model is chosen.
CREATE TABLE IF NOT EXISTS embeddings (
    info_hash     BYTEA PRIMARY KEY REFERENCES torrents (info_hash) ON DELETE CASCADE,
    model         TEXT NOT NULL,
    embedding     vector(768),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
