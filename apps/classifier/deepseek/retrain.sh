#!/bin/bash
# Retrain MLP after targeted labeling
# Run this after label_targeted.sh completes

set -e

MLP_DIR="$(dirname "$0")/../mlp"
cd "$MLP_DIR"

source ../deepseek/venv/bin/activate

echo "=== Retraining MLP ==="
echo ""

# Check current labeled count
echo "Current labeled data:"
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'  Total: {cur.fetchone()[0]}')
    cur.execute('SELECT label_category, COUNT(*) FROM labeled_results GROUP BY label_category ORDER BY COUNT(*) DESC')
    for row in cur.fetchall():
        print(f'  {row[0]}: {row[1]}')
conn.close()
"
echo ""

# Export labeled data
echo "Exporting labeled data..."
python src/export_labeled.py

# Split train/test
echo "Splitting train/test..."
python3 -c "
import json
from collections import Counter

items = []
with open('data/labeled_data/merged.jsonl') as f:
    for line in f:
        if line.strip():
            items.append(json.loads(line))

items.sort(key=lambda x: x['infohash'])
split_idx = int(len(items) * 0.9)
train = items[:split_idx]
test = items[split_idx:]

with open('data/labeled_data/train.jsonl', 'w') as f:
    for item in train:
        f.write(json.dumps(item) + '\n')

with open('data/labeled_data/test.jsonl', 'w') as f:
    for item in test:
        f.write(json.dumps(item) + '\n')

print(f'Train: {len(train)} | Test: {len(test)}')
train_cats = Counter(t['label_category'] for t in train)
for cat, count in sorted(train_cats.items(), key=lambda x: -x[1]):
    print(f'  {cat}: {count} ({count/len(train)*100:.1f}%)')
"

# Retrain
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUT_DIR="data/models/mlp_${TIMESTAMP}"
echo ""
echo "Training model to: $OUT_DIR"
python src/train_mlp.py --data data/labeled_data/train.jsonl --config config/mlp.yaml --out_dir "$OUT_DIR"

echo ""
echo "=== Done ==="
echo "Model saved to: $OUT_DIR"
echo ""
echo "To update web demo, copy the model:"
echo "  cp $OUT_DIR/torrent_classifier.joblib ../mlp/web/models/"
