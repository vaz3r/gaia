-- Migration 001: Add classification columns and indexes to torrents
-- Safe to run on production (uses IF NOT EXISTS)

ALTER TABLE torrents
    ADD COLUMN IF NOT EXISTS category VARCHAR(32),
    ADD COLUMN IF NOT EXISTS category_confidence REAL,
    ADD COLUMN IF NOT EXISTS needs_review BOOLEAN DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS classified_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS classification_meta JSONB;

-- Fast index for the periodic worker to find unclassified torrents
CREATE INDEX IF NOT EXISTS idx_torrents_unclassified 
    ON torrents (verified_at DESC) 
    WHERE classified_at IS NULL;

-- Fast index for user searches & category filtering by popularity
CREATE INDEX IF NOT EXISTS idx_torrents_category_popularity 
    ON torrents (category, popularity_score DESC) 
    WHERE category IS NOT NULL;

-- Fast index for the human review queue
CREATE INDEX IF NOT EXISTS idx_torrents_review_queue 
    ON torrents (popularity_score DESC) 
    WHERE needs_review = TRUE;
