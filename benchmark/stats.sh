#!/usr/bin/env bash
# Dump the crawler's latest crawl stats + peer failure breakdown from logs.
#
# Usage:
#   benchmark/stats.sh [since] [docker-compose-dir]
#
# Defaults: last 120s, ./crawler compose dir.
set -euo pipefail

SINCE="${1:-120s}"
COMPOSE_DIR="${2:-$(cd "$(dirname "$0")/.." && pwd)/crawler}"

cd "$COMPOSE_DIR"

echo "=== crawl stats (last ${SINCE}) ==="
docker compose logs crawler --since "$SINCE" 2>&1 \
    | grep "crawl stats" | tail -3

echo ""
echo "=== peer failure breakdown (last ${SINCE}) ==="
docker compose logs crawler --since "$SINCE" 2>&1 \
    | grep "peer failure breakdown" | tail -1
