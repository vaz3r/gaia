-- Migration: 0013_dashboard_perf_indexes.sql
-- Description: Performance indexes for dashboard API queries.
-- All indexes built CONCURRENTLY — zero table locks, safe to run while crawler is live.
-- Each index takes ~30-60s to build on 3M rows; progress visible in pg_stat_progress_create_index.

-- 1. Partial index covering blocked/suppressed torrents for GET /api/scoring/blocked.
--    Eliminates the 3,500ms+ parallel seq scan for COUNT(*) and the main listing query.
--    Covers: WHERE (policy_action = 'SUPPRESS' OR risk_tier = 'BLOCKED')
--    with ORDER BY scored_at DESC NULLS LAST, verified_at DESC
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_blocked
    ON torrents (scored_at DESC NULLS LAST, verified_at DESC)
    WHERE (policy_action = 'SUPPRESS' OR risk_tier = 'BLOCKED');

-- 2. Composite index for risk_tier filter + verified_at sort in GET /api/torrents.
--    Eliminates the 666ms scan (30k rows skipped to find 25 matching).
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_risk_tier_verified
    ON torrents (risk_tier, verified_at DESC);

-- 3. Composite index for policy_action filter + verified_at sort in GET /api/torrents.
--    Eliminates the 122ms scan for SUPPRESS/DOWNRANK/ALLOW/REVIEW filters.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_policy_action_verified
    ON torrents (policy_action, verified_at DESC);
