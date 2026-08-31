# Torrent Classifier - System Documentation

**Last Updated:** 2026-08-31
**Status:** Active development - Manual labeling phase

---

## 1. Overview

The classifier is a fine-tuned transformer model that categorizes BitTorrent metadata into 8 content categories:

| Category | Description | Production Use |
|---|---|---|
| **Anime** | Japanese animation (fansub tags, franchise names) | ✅ |
| **Applications** | Software, tools, installers | ✅ |
| **Documentaries** | Factual/non-fiction (BBC, PBS, NatGeo) | ✅ |
| **Games** | Scene releases, console ROMs | ✅ |
| **Movies** | Feature films (quality + year tags) | ✅ |
| **Music** | Albums, discographies, FLAC/MP3 | ✅ |
| **Television** | Episodic live-action (SxxExx format) | ✅ |
| **Other** | Porn, spam, ambiguous, low-signal | ✅ (filter) |

**Note:** Porn content is mapped to "Other" in production. The model does not output "Porn" as a separate category.

---

## 2. Architecture

```
Input (torrent metadata)
    ↓
text_builder.py (feature extraction + text formatting)
    ↓
sentence-transformers/all-MiniLM-L12-v2 (fine-tuned)
    ↓
INT8 ONNX Runtime (CPU, ~43 it/s)
    ↓
Category prediction + confidence
```

### Key Components

- **`src/core/text_builder.py`** - Builds plain-text representation from torrent metadata (name, files, size). Includes regex-based feature detection for anime fansubs, game scene tags, app vendors, etc.
- **`src/train_production.py`** - Training pipeline with balanced loss weighting and sample-weighted cross-entropy.
- **`src/classify_batch.py`** - Batch classification CLI (supports embedding, transformer, and LLM modes).
- **`src/export_onnx.py`** - Exports PyTorch model to quantized INT8 ONNX.

---

## 3. Current Performance

### Balanced Benchmark (1,650 samples)

| Category | F1 Score | Status |
|---|---|---|
| Music | 0.943 | ✅ Production grade |
| Documentaries | 0.945 | ✅ Production grade |
| Television | 0.869 | ✅ Production grade |
| Movies | 0.819 | ✅ Production grade |
| Other | 0.790 | ⚠️ OK |
| **Games** | **0.659** | ❌ Needs improvement |
| **Anime** | **0.537** | ❌ Needs improvement |
| **Applications** | **0.494** | ❌ Needs improvement |

**Overall:** 77.7% accuracy | **Macro F1: 0.757**

### Key Confusion Clusters

1. **Anime ↔ Television** (58 misclassified): Japanese anime with Western season format (S01) lacks fansub tags
2. **Movies → Anime** (60 misclassified): Movies being pushed to Anime
3. **Other → Anime** (38 misclassified): False positives from anime detector
4. **Applications ↔ Games** (51 misclassified): Shared vocabulary (setup.exe, crack, patch)

### Root Causes

- **Data imbalance**: Anime has 322 training samples (3.5%), Other has 3,915 (42.2%)
- **Text builder limitations**: Regex-based feature detection is too coarse for edge cases
- **Cross-category vocabulary**: "crack", "patch", "setup.exe" appear in both Games and Applications

---

## 4. Training Data

### Current Pool (9,280 items)

| Source | Items | Notes |
|---|---|---|
| `baseline_v2/train.jsonl` | 3,953 | Original training set |
| `subagent_annotated.jsonl` | 5,327 | Sub-agent labeled (lower quality) |
| **Total** | **9,280** | |

### Class Distribution (Current)

| Category | Count | % |
|---|---|---|
| Other | 3,915 | 42.2% |
| Television | 1,652 | 17.8% |
| Movies | 1,009 | 10.9% |
| Games | 774 | 8.3% |
| Music | 727 | 7.8% |
| Applications | 709 | 7.6% |
| Anime | 322 | 3.5% |
| Documentaries | 172 | 1.9% |

### Evaluation Sets

| File | Samples | Purpose |
|---|---|---|
| `manual_eval_set_balanced_2000.jsonl` | 1,650 | Balanced benchmark (primary) |
| `manual_eval_set_1000.jsonl` | 1,000 | Natural distribution test |
| `baseline_v2/validation.jsonl` | 627 | Held-out validation split |

---

## 5. Manual Labeling Pipeline (New)

### Goal
Label **5,000+ new items per category** using AI chat (Google AI Studio / DeepSeek Chat) to achieve 95%+ F1 on all categories.

### Folder Structure

```
labeling/
├── SYSTEM_PROMPT.md          # System prompt for AI chat
├── HOW_TO_LABEL.md           # Step-by-step instructions
├── extract_batches.py        # PostgreSQL extraction script
├── merge_labeled.py          # Merge all labeled batches
├── batches/                  # Unlabeled batches (100 items each)
│   ├── anime/               (80 batches, 8,000 items)
│   ├── applications/        (98 batches, 9,177 items)
│   ├── documentaries/       (34 batches, 3,239 items)
│   ├── games/               (213 batches, 21,300 items)
│   ├── movies/              (51 batches, 5,100 items)
│   ├── music/               (50 batches, 5,000 items)
│   └── television/          (53 batches, 5,300 items)
└── labeled/                  # User-labeled results (append here)
```

### Workflow

1. Open AI chat (Google AI Studio / DeepSeek Chat)
2. Paste `labeling/SYSTEM_PROMPT.md` as system prompt
3. Open a batch file from `labeling/batches/{category}/`
4. Copy JSON array, paste into AI chat
5. Copy AI response, save to `labeling/labeled/{category}/batch_XXX_labeled.json`
6. Repeat for all batches
7. Run `python3 labeling/merge_labeled.py` to combine

### Extraction Queries

Batches are extracted from PostgreSQL (`workspace-production:5432/craw`) using targeted SQL queries:

- **Anime**: Fansub tag regex + franchise name matching
- **Applications**: Software vendor names (Adobe, Autodesk, JetBrains, etc.)
- **Games**: Scene group tags + console ROM extensions
- **Documentaries**: Documentary markers (BBC, PBS, NOVA, etc.)
- **Music**: Discography/album/FLAC keywords
- **Movies**: Quality tags + year, excluding episode markers
- **Television**: SxxExx / Season format

All batches exclude infohashes already in existing training data.

---

## 6. Database

### Connection

```
Host: workspace-production
Port: 5432
Database: craw
User: crawler
```

### Schema (torrents table)

| Column | Type | Description |
|---|---|---|
| infohash | bytea | Unique torrent identifier |
| name | text | Torrent name |
| piece_length | bigint | Piece size |
| total_size | bigint | Total size in bytes |
| file_count | integer | Number of files |
| files | jsonb | File paths and sizes |
| fetch_attempts | integer | Fetch retry count |
| verified_at | timestamptz | Verification timestamp |

**Total torrents:** ~1.47M

---

## 7. Configuration

### Training Config (`train_production.py`)

```python
MODEL = "sentence-transformers/all-MiniLM-L12-v2"
MAX_LENGTH = 256
BATCH_SIZE = 32
LEARNING_RATE = 2.5e-5
EPOCHS = 3
WARMUP_RATIO = 0.10
```

### Inference Config (`config/transformer.yaml`)

```yaml
confidence_threshold: 0.45  # Low-confidence non-Other → Other
```

---

## 8. Deployment

### Docker

```yaml
# docker-compose.yml
ports:
  - "6881:6881/udp"  # DHT listener
```

### ONNX Model

- **Path:** `data/models/transformer/single_stage/model_int8.onnx`
- **Format:** INT8 quantized ONNX
- **Throughput:** ~43 items/sec (CPU)
- **Latency:** <25ms per item

---

## 9. Next Steps

1. **Complete manual labeling** (~35 hours of work)
2. **Merge labeled data** with existing training pool
3. **Retrain model** with balanced 40k+ dataset
4. **Evaluate** on all 4 benchmarks
5. **Target:** Macro F1 ≥ 0.85, per-class F1 ≥ 0.80
6. **Deploy** updated ONNX model to production

---

## 10. File Reference

### Core Source Files

| File | Purpose |
|---|---|
| `src/core/text_builder.py` | Text representation builder |
| `src/train_production.py` | Training pipeline |
| `src/classify_batch.py` | Batch classification CLI |
| `src/export_onnx.py` | ONNX export |
| `src/evaluate.py` | Evaluation harness |
| `src/generate_pseudo_labels.py` | Pseudo-label generation |
| `src/label_al_candidates.py` | Active learning labeler |

### Data Files

| File | Purpose |
|---|---|
| `data/training_combined_v10_true.jsonl` | Canonical training set |
| `data/manual_eval_set_balanced_2000.jsonl` | Balanced benchmark |
| `data/manual_eval_set_1000.jsonl` | Natural distribution test |
| `data/baseline_v2/train.jsonl` | Original training data |
| `data/baseline_v2/validation.jsonl` | Validation split |
| `data/label_map.py` | Label mapping (1000 items) |
| `data/LABELING_INSTRUCTIONS.md` | Labeling guidelines |
