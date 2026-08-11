#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE="$(dirname "$SCRIPT_DIR")"
BIN="${WORKSPACE}/target/release/dht-crawler"
DB="${SCRIPT_DIR}/crawler.sqlite"
STATE="${SCRIPT_DIR}/state"

mkdir -p "$STATE"

AGGRESSIVE=""
if [[ "${1:-}" == "--aggressive" ]]; then
    AGGRESSIVE="--aggressive"
    echo "Starting dht-crawler (AGGRESSIVE MODE)"
else
    echo "Starting dht-crawler"
    echo "  Tip: run with --aggressive for VPS-optimized settings"
fi

echo "  DB:    $DB"
echo "  State: $STATE"
echo ""
echo "Press Ctrl-C to stop gracefully."

exec "$BIN" run \
  --db "$DB" \
  --state-dir "$STATE" \
  --port 6881 \
  --min-seen 1 \
  $AGGRESSIVE \
  --log dht_crawler=debug
