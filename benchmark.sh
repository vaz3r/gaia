#!/usr/bin/env bash
set -euo pipefail

DURATION_SECS="${1:-180}"
LABEL="${2:-Benchmark}"

echo "========================================================"
echo " Starting $LABEL (${DURATION_SECS}s measurement window) "
echo "========================================================"

# Initial DB count
INITIAL_COUNT=$(docker compose exec -T postgres psql -U crawler -d crawler -t -A -c "SELECT COUNT(*) FROM torrents;")
START_TIME=$(date +%s)

echo "Initial indexed torrents: $INITIAL_COUNT"
echo "Waiting ${DURATION_SECS}s..."

sleep "$DURATION_SECS"

END_TIME=$(date +%s)
FINAL_COUNT=$(docker compose exec -T postgres psql -U crawler -d crawler -t -A -c "SELECT COUNT(*) FROM torrents;")
ELAPSED=$((END_TIME - START_TIME))
DIFF=$((FINAL_COUNT - INITIAL_COUNT))

RATE_PER_HR=$(awk "BEGIN {printf \"%.2f\", ($DIFF / $ELAPSED) * 3600}")

echo "--------------------------------------------------------"
echo " Result for $LABEL:"
echo " Initial: $INITIAL_COUNT | Final: $FINAL_COUNT | New: +$DIFF"
echo " Elapsed: ${ELAPSED}s | Verified Rate: ${RATE_PER_HR} torrents/hour"
echo "--------------------------------------------------------"

# Output crawler latest stats line
echo "Crawler live stats:"
docker compose logs crawler --tail 2 | grep -v 'health' || true
echo "========================================================"
