#!/usr/bin/env bash
# Measure torrent discovery rate from the SQLite database over a window.
#
# Usage:
#   benchmark/torrents_rate.sh [seconds] [docker-compose-dir] [db-container-path]
#
# Defaults: 600s window, ./crawler compose dir, /data/crawler.sqlite.
# Prints torrents found in the window and the implied rate (per hr / per day).
set -euo pipefail

WINDOW="${1:-600}"
COMPOSE_DIR="${2:-$(cd "$(dirname "$0")/.." && pwd)/crawler}"
CONTAINER="crawler"
DB_PATH="${3:-/data/crawler.sqlite}"

cd "$COMPOSE_DIR"

count_torrents() {
    docker cp "$CONTAINER:$DB_PATH" /tmp/bench_torrents.sqlite >/dev/null 2>&1 \
        || { echo "ERROR: cannot copy $DB_PATH from $CONTAINER (is the stack up?)" >&2; exit 1; }
    python3 -c "import sqlite3; print(sqlite3.connect('file:/tmp/bench_torrents.sqlite?mode=ro',uri=True).execute('SELECT COUNT(*) FROM torrents').fetchone()[0])"
}

echo "Counting torrents, sampling for ${WINDOW}s..."
before="$(count_torrents)"
sleep "$WINDOW"
after="$(count_torrents)"

delta=$(( after - before ))
per_hr=$(awk -v d="$delta" -v t="$WINDOW" 'BEGIN{printf "%.1f", d/t*3600}')
per_day=$(awk -v d="$delta" -v t="$WINDOW" 'BEGIN{printf "%.1f", d/t*86400}')

cat <<EOF
=== Torrent rate ($WINDOW s window) ===
  Before: $before  After: $after
  Found in window : $delta
  Rate            : ${per_hr}/hr = ${per_day}/day
EOF
