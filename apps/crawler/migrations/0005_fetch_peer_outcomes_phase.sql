-- Add phase and elapsed_ms columns to fetch_peer_outcomes for separating
-- connect-phase and metadata-phase outcomes. Existing rows have NULL values;
-- analysis queries must filter WHERE phase IS NOT NULL.
ALTER TABLE fetch_peer_outcomes
  ADD COLUMN IF NOT EXISTS phase TEXT,
  ADD COLUMN IF NOT EXISTS elapsed_ms INTEGER;

CREATE INDEX IF NOT EXISTS idx_fpo_phase_created
  ON fetch_peer_outcomes (phase, created_at);
