-- 0003_failure_time_index.sql — support time-range failure breakdowns.
-- The admin API's failures endpoint aggregates `scanned` by failure_reason over
-- a time window; without an index on last_attempt that scans all 7.6M+ rows and
-- hits statement_timeout. This index makes range queries bounded.

CREATE INDEX IF NOT EXISTS scanned_last_attempt_idx ON scanned (last_attempt);
