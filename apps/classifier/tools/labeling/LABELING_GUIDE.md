# Targeted Labeling Guide

## Overview

This guide explains how to label specific torrent categories to improve classifier accuracy. The database has ~2M unclassified torrents, but random sampling underrepresents certain categories. Targeted labeling focuses on the weakest categories.

## Why Target Specific Categories?

| Category | Real-World % | Training % | Low Conf % | Issue |
|----------|-------------|------------|------------|-------|
| Documentaries | 0.6% | 9.2% | 45.1% | Most confused |
| Other | 9.7% | 12.0% | 12.3% | Weakest F1 (0.730) |
| Anime | 2.8% | 9.5% | 9.0% | Underrepresented |
| Applications | 2.7% | 8.5% | 9.5% | Underrepresented |

## Quick Start

### Step 1: Label 10K Targeted Torrents

```bash
cd apps/classifier/deepseek
./label_targeted.sh
```

This labels 2,500 torrents from each of 4 categories:
- Documentaries (most confused — 45.1% low confidence)
- Other (weakest F1 — catch-all bucket)
- Anime (underrepresented in database)
- Applications (underrepresented in database)

**Estimated time:** ~2 hours

### Step 2: Retrain the Model

```bash
cd apps/classifier/deepseek
./retrain.sh
```

This:
1. Exports labeled data from PostgreSQL
2. Splits into train/test (90/10)
3. Trains new MLP model
4. Shows category distribution

### Step 3: Test the New Model (Optional)

```bash
cd apps/classifier/mlp
source ../deepseek/venv/bin/activate
python test_10k.py
```

## Manual Usage

### Label a Specific Category

```bash
cd apps/classifier/deepseek
source venv/bin/activate

# Label 2,500 Documentaries (50 batches of 50)
python classify.py --batch 50 --loops 50 --target Documentaries --delay 10

# Label 2,500 Other
python classify.py --batch 50 --loops 50 --target Other --delay 10

# Label 2,500 Anime
python classify.py --batch 50 --loops 50 --target Anime --delay 10

# Label 2,500 Applications
python classify.py --batch 50 --loops 50 --target Applications --delay 10
```

### Label Low-Confidence Items

```bash
# From a previous 50K test
cd apps/classifier/mlp
python3 -c "
import json
results = []
with open('data/test_results/test_50k_results.jsonl') as f:
    for line in f:
        if line.strip():
            results.append(json.loads(line))
very_low = [r for r in results if r['confidence'] < 0.5]
with open('data/test_results/very_low_infohashes.txt', 'w') as f:
    for r in very_low:
        f.write(r['infohash'] + '\n')
print(f'Exported {len(very_low)} very-low confidence infohashes')
"

cd ../deepseek
python classify.py --file ../mlp/data/test_results/very_low_infohashes.txt --batch 50 --delay 10
```

### Retrain Manually

```bash
cd apps/classifier/mlp
source ../deepseek/venv/bin/activate

# Export labeled data
python src/export_labeled.py

# Split train/test
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
"

# Train model
python src/train_mlp.py --data data/labeled_data/train.jsonl --config config/mlp.yaml --out_dir data/models/mlp_$(date +%Y%m%d)
```

## Available Target Categories

| Flag | Category | Pattern |
|------|----------|---------|
| `--target Adult` | Adult | porn, xxx, hentai, jav, onlyfans, etc. |
| `--target Anime` | Anime | Erai-raws, SubsPlease, Crunchyroll, etc. |
| `--target Applications` | Applications | Adobe, JetBrains, Office, etc. |
| `--target Documentaries` | Documentaries | NatGeo, Discovery, BBC Earth, etc. |
| `--target Games` | Games | FitGirl, CODEX, SKIDROW, etc. |
| `--target Movies` | Movies | BRRip, BluRay, YIFY, 1080p, etc. |
| `--target Music` | Music | FLAC, album, discography, etc. |
| `--target Television` | Television | S01E01, Season, Episode, etc. |
| `--target Other` | Other | ebook, pdf, course, tutorial, etc. |

## Progress Checkpoints

The `label_targeted.sh` script saves checkpoints after each category:

```
After Documentaries: 14032
After Other: 16532
After Anime: 19032
After Applications: 21532
```

If interrupted, you can check progress:

```bash
cd apps/classifier/deepseek
source venv/bin/activate
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
with conn.cursor() as cur:
    cur.execute('SELECT COUNT(*) FROM labeled_results')
    print(f'Total labeled: {cur.fetchone()[0]}')
    cur.execute('SELECT label_category, COUNT(*) FROM labeled_results GROUP BY label_category ORDER BY COUNT(*) DESC')
    for row in cur.fetchall():
        print(f'  {row[0]}: {row[1]}')
conn.close()
"
```

## Logs

Logs are saved to `label_targeted_YYYYMMDD_HHMMSS.log` in the `deepseek/` directory.

To monitor progress:

```bash
tail -f label_targeted_*.log
```

## Troubleshooting

### Database Connection Lost

If the database times out, the script will fail. Wait 30 seconds and retry:

```bash
# Check if DB is back
python3 -c "
import psycopg2
conn = psycopg2.connect(host='workspace-production', port=5432, dbname='craw', user='crawler', password='83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b', connect_timeout=10)
print('DB is back!')
conn.close()
"

# Resume from where it stopped (already-labeled torrents are skipped)
python classify.py --batch 50 --loops 50 --target Documentaries --delay 10
```

### SSL Errors

The improved DeepSeek client handles SSL errors with retries. If persistent:

```bash
# Check DeepSeek session
python3 -c "
from deepseek.auth import get_session
session = get_session(allow_interactive=False)
print(f'Token valid: {session.token[:20]}...')
"
```

### Rate Limiting

The client uses:
- 10 RPM limit
- Retry-After header parsing
- Jittered backoff
- Circuit breaker (25% failure rate → 3min cooldown)

If rate limited, wait 2-3 minutes and retry.

## Current Model Performance

| Model | Training | Avg Conf | Low Conf | Low Conf Rate |
|-------|----------|----------|----------|---------------|
| 7K | 6,629 | 94.4% | 774 | 7.7% |
| 8K | 7,165 | 96.2% | 515 | 5.1% |
| 10K | 9,140 | 96.2% | 2,487 | 5.0% |
| 11K | 10,275 | 96.6% | 234 | 4.7% |

## Files

| File | Description |
|------|-------------|
| `deepseek/label_targeted.sh` | Main labeling script (10K torrents) |
| `deepseek/retrain.sh` | Retraining pipeline |
| `deepseek/classify.py` | DeepSeek classifier with `--target` flag |
| `mlp/test_10k.py` | 50K test evaluation script |
| `mlp/data/models/` | Saved models |
| `mlp/data/labeled_data/` | Training/test data |
| `mlp/data/test_results/` | Test results and low-confidence exports |
