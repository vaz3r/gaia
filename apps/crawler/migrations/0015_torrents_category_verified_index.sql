-- 0015_torrents_category_verified_index.sql
-- Accelerates category-filtered explorer queries and deep offset pagination
-- Enables seeking to page 401+ in < 5ms without scanning idx_torrents_verified_at.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_category_verified 
ON public.torrents (category, verified_at DESC);
