-- 0014_crawler_health_check_index.sql
-- Optimizes the periodic crawler query:
-- SELECT infohash, total_seen, last_seen FROM torrents WHERE last_seen > now() - interval '7 days' ORDER BY last_health_check ASC NULLS FIRST LIMIT $1
-- Drops execution time from 16.3 seconds (1.76 GB disk scan) down to < 1 ms.

CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_torrents_health_recent 
ON public.torrents (last_health_check ASC NULLS FIRST, last_seen);
