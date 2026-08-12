#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE="$(dirname "$SCRIPT_DIR")"
BIN="${WORKSPACE}/target/release/crawler"
DB="${SCRIPT_DIR}/crawler.sqlite"
STATE="${SCRIPT_DIR}/state"
LOG="${SCRIPT_DIR}/crawl.log"

mkdir -p "$STATE"

AGGRESSIVE=""
case "${1:-}" in
    --purge|purge)
        echo "Purging crawl data (db + routing state)..."
        if [[ "${2:-}" == "--yes" ]]; then
            exec "$BIN" purge --db "$DB" --state-dir "$STATE" --yes
        else
            exec "$BIN" purge --db "$DB" --state-dir "$STATE"
        fi
        ;;
    --aggressive)
        AGGRESSIVE="--aggressive"
        echo "Starting crawler (AGGRESSIVE MODE)"
        ;;
    *)
        echo "Starting crawler"
        echo "  Tip: run with --aggressive for VPS-optimized settings, or --purge to wipe data"
        ;;
esac

echo "  DB:    $DB"
echo "  State: $STATE"
echo "  Log:   $LOG"
echo ""
echo "Press Ctrl-C to stop gracefully."

exec "$BIN" run \
  --db "$DB" \
  --state-dir "$STATE" \
  --port 6881 \
  $AGGRESSIVE \
  --log crawler=info \
  >"$LOG" 2>&1
