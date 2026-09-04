-- Add first_seen, last_seen, and total_seen to torrents table
ALTER TABLE torrents
    ADD COLUMN IF NOT EXISTS first_seen TIMESTAMPTZ DEFAULT now(),
    ADD COLUMN IF NOT EXISTS last_seen TIMESTAMPTZ DEFAULT now(),
    ADD COLUMN IF NOT EXISTS total_seen BIGINT DEFAULT 1;
