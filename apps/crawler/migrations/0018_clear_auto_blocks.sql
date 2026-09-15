-- Migration 0018: Clear all automatically-generated surveillance blocks.
-- Automatic blocking has been removed from the crawler. is_blocked = TRUE is now
-- exclusively reserved for operator-reviewed manual blocks set via the dashboard toggle.
-- This migration resets the slate: all previously auto-set blocks are cleared.
UPDATE dht_surveillance_nodes SET is_blocked = FALSE WHERE is_blocked = TRUE;
