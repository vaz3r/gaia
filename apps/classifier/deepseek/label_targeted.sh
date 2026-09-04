#!/bin/bash
# Targeted labeling script for 4 categories
# Each category: 2,500 torrents (50 batches of 50)
# Total: 10,000 torrents
# Estimated time: ~2 hours

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

source venv/bin/activate

LOG_FILE="label_targeted_$(date +%Y%m%d_%H%M%S).log"
echo "Starting targeted labeling at $(date)" | tee "$LOG_FILE"

# Check DB connection first
echo "Checking database connection..." | tee -a "$LOG_FILE"
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'Current labeled: {cur.fetchone()[0]}')
conn.close()
" 2>&1 | tee -a "$LOG_FILE"

# Label Documentaries (2,500)
echo "" | tee -a "$LOG_FILE"
echo "=== Labeling Documentaries (2,500) ===" | tee -a "$LOG_FILE"
python classify.py --batch 50 --loops 50 --target Documentaries --delay 10 2>&1 | tee -a "$LOG_FILE"

# Checkpoint
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'After Documentaries: {cur.fetchone()[0]}')
conn.close()
" 2>&1 | tee -a "$LOG_FILE"

# Label Other (2,500)
echo "" | tee -a "$LOG_FILE"
echo "=== Labeling Other (2,500) ===" | tee -a "$LOG_FILE"
python classify.py --batch 50 --loops 50 --target Other --delay 10 2>&1 | tee -a "$LOG_FILE"

# Checkpoint
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'After Other: {cur.fetchone()[0]}')
conn.close()
" 2>&1 | tee -a "$LOG_FILE"

# Label Anime (2,500)
echo "" | tee -a "$LOG_FILE"
echo "=== Labeling Anime (2,500) ===" | tee -a "$LOG_FILE"
python classify.py --batch 50 --loops 50 --target Anime --delay 10 2>&1 | tee -a "$LOG_FILE"

# Checkpoint
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'After Anime: {cur.fetchone()[0]}')
conn.close()
" 2>&1 | tee -a "$LOG_FILE"

# Label Applications (2,500)
echo "" | tee -a "$LOG_FILE"
echo "=== Labeling Applications (2,500) ===" | tee -a "$LOG_FILE"
python classify.py --batch 50 --loops 50 --target Applications --delay 10 2>&1 | tee -a "$LOG_FILE"

# Final count
echo "" | tee -a "$LOG_FILE"
echo "=== Done ===" | tee -a "$LOG_FILE"
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'Final labeled: {cur.fetchone()[0]}')
    cur.execute('SELECT label_category, COUNT(*) FROM labeled_results GROUP BY label_category ORDER BY COUNT(*) DESC')
    for row in cur.fetchall():
        print(f'  {row[0]}: {row[1]}')
conn.close()
" 2>&1 | tee -a "$LOG_FILE"

echo "" | tee -a "$LOG_FILE"
echo "Completed at $(date)" | tee -a "$LOG_FILE"
echo "Log saved to: $LOG_FILE" | tee -a "$LOG_FILE"
