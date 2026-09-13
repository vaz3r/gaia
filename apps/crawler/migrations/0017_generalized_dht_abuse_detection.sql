-- Migration 0017: Generalized DHT Abuse & Non-Contributing Spy Detection
ALTER TABLE dht_surveillance_nodes
ADD COLUMN IF NOT EXISTS find_node_count BIGINT NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS get_peers_count BIGINT NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS announce_peer_count BIGINT NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS distinct_node_ids INT NOT NULL DEFAULT 1,
ADD COLUMN IF NOT EXISTS abuse_category TEXT NOT NULL DEFAULT 'Unclassified';

CREATE INDEX IF NOT EXISTS idx_surveillance_category ON dht_surveillance_nodes (abuse_category);
CREATE INDEX IF NOT EXISTS idx_surveillance_node_ids ON dht_surveillance_nodes (distinct_node_ids DESC);
