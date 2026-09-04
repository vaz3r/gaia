-- Migration 0007: Recycle dead unverified jobs back to pending with staggered retry intervals.
-- Torrents that have not been verified into the torrents table will be given retry attempts.
UPDATE verification_jobs vj
SET status = 'pending',
    retry_count = 0,
    next_retry_at = now() + (random() * interval '30 minutes'),
    updated_at = now()
WHERE vj.status = 'dead'
  AND NOT EXISTS (
      SELECT 1 FROM torrents t WHERE t.infohash = vj.infohash
  );
