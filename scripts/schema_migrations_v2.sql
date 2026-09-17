-- ============================================================================
-- WARNING: HISTORICAL / UNAPPLIED ARTIFACT — DO NOT RUN
-- ============================================================================
-- This migration is a historical record from the v2-scaling architecture branch.
-- It must NOT be applied to current or future environments.
--
-- It creates objects that have been explicitly retired:
--   - torrents.updated_at trigger (touch_updated_at) — not used by current apps
--   - portal_sync_state table — superseded by direct Meilisearch sync in apps/sync
--
-- This file is retained for audit trail purposes only.
-- Pending archival/removal after the dedicated bootstrap/migration audit.
-- ============================================================================

SET lock_timeout = '5s';

-- 1. Universal change-detection column
ALTER TABLE torrents ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

-- 2. Discriminating trigger (metadata/moderation only, health/scores excluded)
CREATE OR REPLACE FUNCTION touch_updated_at() RETURNS trigger AS $$
BEGIN
  IF (NEW.name, NEW.category, NEW.total_size, NEW.file_count,
      NEW.policy_action, NEW.risk_tier, NEW.verified_at)
     IS DISTINCT FROM
     (OLD.name, OLD.category, OLD.total_size, OLD.file_count,
      OLD.policy_action, OLD.risk_tier, OLD.verified_at)
  THEN 
    NEW.updated_at = now();
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_torrents_touch ON torrents;
CREATE TRIGGER trg_torrents_touch
  BEFORE UPDATE ON torrents
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 3. Dead-torrent decay tracking columns
ALTER TABLE torrents ADD COLUMN IF NOT EXISTS last_health_attempt timestamptz;
ALTER TABLE torrents ADD COLUMN IF NOT EXISTS health_fail_streak int NOT NULL DEFAULT 0;
ALTER TABLE torrents ADD COLUMN IF NOT EXISTS last_decay_sweep timestamptz;

-- 4. Durable sync state table
DROP TABLE IF EXISTS portal_sync_state;
CREATE TABLE portal_sync_state (
  loop_name     text PRIMARY KEY,   -- 'backfill' | 'fast' | 'health' | 'suppression' | 'decay'
  cursor_ts     timestamptz,
  cursor_hash   text,
  completed     boolean NOT NULL DEFAULT false,
  rows_synced   bigint  NOT NULL DEFAULT 0,
  last_run_at   timestamptz,
  last_error    text
);

-- 5. Seed portal_sync_state idempotently (incremental loops start at now(), backfill starts at NULL)
INSERT INTO portal_sync_state (loop_name, cursor_ts, cursor_hash, completed, rows_synced, last_run_at)
VALUES 
  ('fast', now(), '', false, 0, now()),
  ('suppression', now(), '', false, 0, now()),
  ('health', now(), '', false, 0, now()),
  ('decay', now(), '', false, 0, now()),
  ('backfill', NULL, '', false, 0, NULL)
ON CONFLICT (loop_name) DO NOTHING;
