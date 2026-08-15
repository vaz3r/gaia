#!/usr/bin/env bash
# Failure economics analysis for the crawler retry rework.
#
# Reads the live Postgres `scanned` table and reports per-failure-class
# conversion rates and retry economics, so retry caps stay data-driven.
#
# Usage:
#   benchmark/failures_analysis.sh [hours]
# Requires: docker compose stack up, POSTGRES_PASSWORD in .env
set -euo pipefail

HOURS="${1:-48}"
PGPASSWORD="$(grep '^POSTGRES_PASSWORD=' .env | cut -d= -f2)"

run_sql() {
    docker run --rm -i --network gaia_gaia -e PGPASSWORD="$PGPASSWORD" \
        pgvector/pgvector:pg16 psql "postgres://crawler:${PGPASSWORD}@postgres:5432/crawler" \
        -v ON_ERROR_STOP=1 -t -A "$@"
}

echo "=== Retry economics (last ${HOURS}h): failed-work vs conversions per attempt level ==="
run_sql -c "
SELECT attempts,
       COUNT(*) FILTER (WHERE status='failed') AS failed_work,
       COUNT(*) FILTER (WHERE status='ok') AS conversions,
       CASE WHEN COUNT(*) FILTER (WHERE status='failed')>0
            THEN ROUND(100.0*COUNT(*) FILTER (WHERE status='ok')/(COUNT(*) FILTER (WHERE status='failed')+COUNT(*) FILTER (WHERE status='ok')),4)
            ELSE 0 END AS conv_rate_pct
FROM scanned
WHERE last_attempt >= EXTRACT(EPOCH FROM now()-interval '${HOURS} hours')::bigint
GROUP BY attempts ORDER BY attempts;" | column -t -s'|'

echo ""
echo "=== Conversion success rate per failure class (attempts 2-5, last ${HOURS}h) ==="
run_sql -c "
WITH t AS (
  SELECT COALESCE(failure_reason,'unknown') AS reason, attempts, status, COUNT(*) AS n
  FROM scanned
  WHERE last_attempt >= EXTRACT(EPOCH FROM now()-interval '${HOURS} hours')::bigint
  GROUP BY 1,2,3
)
SELECT reason,
       SUM(n) FILTER (WHERE status='ok') AS verified,
       SUM(n) FILTER (WHERE status='failed') AS failed
FROM t WHERE attempts BETWEEN 2 AND 5
GROUP BY 1 ORDER BY verified DESC LIMIT 15;" | column -t -s'|'

echo ""
echo "=== Current failure distribution (last ${HOURS}h) ==="
run_sql -c "
SELECT COALESCE(failure_reason,'NULL') AS reason, status, COUNT(*) AS n
FROM scanned
WHERE last_attempt >= EXTRACT(EPOCH FROM now()-interval '${HOURS} hours')::bigint
GROUP BY 1,2 ORDER BY n DESC LIMIT 12;" | column -t -s'|'
