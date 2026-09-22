-- Migration: 0003_backfill_health_scores.sql
-- Description: Seed health_scores from torrents table for immediate display.
-- Safety: ON CONFLICT DO NOTHING — won't overwrite existing canonical scores.
-- Applied via: psql (transactional)
-- NOTE: Large INSERT on production. Run during low-traffic window.

INSERT INTO health_scores (infohash, health_score, health_state, confidence, algorithm_version, health_calculated_at)
SELECT
    encode(t.infohash, 'hex'),
    t.health_score,
    CASE
        WHEN t.health_score IS NULL THEN 'UNKNOWN'
        WHEN t.health_score >= 70 THEN 'VERIFIED'
        WHEN t.health_score >= 25 THEN 'UNVERIFIED'
        WHEN t.health_score > 0 THEN 'STALE'
        ELSE 'UNKNOWN'
    END,
    CASE
        WHEN t.health_score IS NULL THEN 0.0
        WHEN t.health_score >= 70 THEN 0.8
        WHEN t.health_score >= 25 THEN 0.5
        WHEN t.health_score > 0 THEN 0.3
        ELSE 0.0
    END,
    'backfill_v1',
    NOW()
FROM torrents t
WHERE t.health_score IS NOT NULL
ON CONFLICT (infohash) DO NOTHING;
