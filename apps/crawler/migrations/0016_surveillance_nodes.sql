-- Migration 0016: DHT Surveillance & Anti-Abuse Tracking
CREATE TABLE IF NOT EXISTS dht_surveillance_nodes (
    ip INET PRIMARY KEY,
    asn TEXT,
    org TEXT,
    score INT NOT NULL DEFAULT 0,
    query_count BIGINT NOT NULL DEFAULT 1,
    distinct_hashes INT NOT NULL DEFAULT 1,
    bep42_violations INT NOT NULL DEFAULT 0,
    bep42_compliant_count INT NOT NULL DEFAULT 0,
    suspected_entity TEXT NOT NULL DEFAULT 'Unknown Monitor',
    sample_hashes BYTEA[] DEFAULT '{}',
    is_blocked BOOLEAN NOT NULL DEFAULT TRUE,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_surveillance_score_last_seen ON dht_surveillance_nodes (score DESC, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_surveillance_last_seen ON dht_surveillance_nodes (last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_surveillance_blocked ON dht_surveillance_nodes (is_blocked) WHERE is_blocked = TRUE;
