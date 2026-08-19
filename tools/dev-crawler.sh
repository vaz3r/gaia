#!/usr/bin/env bash
# Fast crawler iteration loop for the dev overlay.
#
# Usage:
#   ./tools/dev-crawler.sh build        # build release + sync binary to volume + restart
#   ./tools/dev-crawler.sh restart      # just restart from current volume binary
#   ./tools/dev-crawler.sh bench [DB]   # 2-minute rate check (default crawler_bench4)
#   ./tools/dev-crawler.sh tail         # follow crawler logs
set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE="docker compose -f docker-compose.yml -f docker-compose.dev.yml"

case "${1:-build}" in
  build)
    (cd crawler && "$HOME/.cargo/bin/cargo" build --release -p crawler)
    docker rm -f crawler-bin-loader >/dev/null 2>&1 || true
    docker run --rm -d --name crawler-bin-loader -v crawler-dev-bin:/data alpine sleep 300 >/dev/null
    sleep 1
    docker cp target/release/crawler crawler-bin-loader:/data/bin/crawler
    docker rm -f crawler-bin-loader >/dev/null 2>&1 || true
    echo "binary synced"
    $COMPOSE up -d --force-recreate --no-deps crawler
    ;;
  restart)
    $COMPOSE up -d crawler
    ;;
  bench)
    DB="${2:-crawler_bench5}"
    INIT=$($COMPOSE exec -T postgres psql -U crawler -d "$DB" -t -A -c "SELECT count(*) FROM torrents")
    START=$(date +%s)
    echo "initial: $INIT (db=$DB)"
    sleep 120
    FINAL=$($COMPOSE exec -T postgres psql -U crawler -d "$DB" -t -A -c "SELECT count(*) FROM torrents")
    END=$(date +%s)
    RATE=$(awk "BEGIN {printf \"%.1f\", (($FINAL-$INIT) / ($END-$START)) * 3600}")
    echo "final: $FINAL | +$((FINAL-INIT)) in $((END-START))s | ${RATE} torrents/hr"
    ;;
  tail)
    $COMPOSE logs -f crawler
    ;;
  *)
    echo "usage: $0 {build|restart|bench|tail}"
    exit 1
    ;;
esac
