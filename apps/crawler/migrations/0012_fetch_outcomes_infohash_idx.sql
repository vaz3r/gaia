-- Index for per-infohash aggregation of fetch outcomes (used by health prober)
CREATE INDEX IF NOT EXISTS idx_fpo_infohash
    ON fetch_peer_outcomes (infohash);
