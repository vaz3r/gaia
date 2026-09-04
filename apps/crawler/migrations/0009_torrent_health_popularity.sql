-- Add health and popularity scoring columns to torrents
ALTER TABLE torrents
    ADD COLUMN IF NOT EXISTS health_score SMALLINT DEFAULT 0 CHECK (health_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS popularity_score SMALLINT DEFAULT 0 CHECK (popularity_score BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS swarm_peers INTEGER DEFAULT 0,
    ADD COLUMN IF NOT EXISTS seed_confirmed BOOLEAN DEFAULT false,
    ADD COLUMN IF NOT EXISTS last_health_check TIMESTAMPTZ;

-- Indices for fast sorting and prober queue selection
CREATE INDEX IF NOT EXISTS idx_torrents_health ON torrents (health_score DESC);
CREATE INDEX IF NOT EXISTS idx_torrents_popularity ON torrents (popularity_score DESC);
CREATE INDEX IF NOT EXISTS idx_torrents_health_check ON torrents (last_health_check ASC NULLS FIRST);
