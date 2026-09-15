-- Migration 0018: Clear all automatically-generated surveillance blocks.
-- Automatic blocking has been removed from the crawler and dashboard. is_blocked = TRUE is now
-- exclusively reserved for operator-reviewed manual blocks set via the dashboard toggle.
-- This migration resets the slate: default becomes FALSE and all previously auto-set blocks are cleared.
ALTER TABLE dht_surveillance_nodes ALTER COLUMN is_blocked SET DEFAULT FALSE;
UPDATE dht_surveillance_nodes SET is_blocked = FALSE WHERE is_blocked = TRUE;
