# Gaia Torrent Classifier

High-throughput, calibrated machine learning classifier and interactive audit studio for DHT crawled torrents. Built to classify 10M+ torrents with zero LLM fallback, sub-millisecond vector inference, and calibrated confidence-margin rejection.

---

## 1. System Architecture

```
                                  ┌────────────────────────┐
                                  │   PostgreSQL (craw)    │
                                  │  2.33M+ Torrents       │
                                  │  (Strict READ_ONLY)    │
                                  └───────────┬────────────┘
                                              │
                    ┌─────────────────────────┴─────────────────────────┐
                    ▼                                                   ▼
       ┌─────────────────────────┐                         ┌─────────────────────────┐
       │   scripts/stream_       │                         │       web/app.py        │
       │   classifier.py         │                         │     FastAPI Studio      │
       │   (Shared-Nothing Shard)│                         │  (Vercel / Linear UI)   │
       └────────────┬────────────┘                         └────────────┬────────────┘
                    │                                                   │
                    └─────────────────────────┬─────────────────────────┘
                                              ▼
                             ┌─────────────────────────────────┐
                             │    src/classifier_service.py    │
                             │  • Multi-modal TF-IDF Features  │
                             │  • 10-Class Calibrated SGD      │
                             │  • Dual Rejection Gates         │
                             └─────────────────────────────────┘
                                              │
                      ┌───────────────────────┴───────────────────────┐
                      ▼                                               ▼
             [ ✓ ACCEPTED ]                                  [ ⚠️ NEEDS REVIEW ]
        Confidence >= 0.65 and                          Confidence < 0.65 (Low Conf)
        Margin >= 0.35                                  or Margin < 0.35 (Ambiguous)
```

### Key Technical Decisions:
- **Zero LLM Fallback**: Rather than sending ambiguous cases to expensive or slow LLMs, unclassifiable or contested torrents are safely routed to human review with diagnostic flags.
- **Open-Set Rejection (`Other` Removed)**: `Other` was removed as a training target class to prevent catch-all misdirection. Out-of-distribution items are rejected using calibrated probability and margin thresholds.
- **Multi-Modal Feature Extraction**: Combines character n-grams (3-5), word n-grams (1-2), log-size, log-file-count, extension byte ratios across 8 categories (video, audio, audiobook, ebook, archive, software, roms), and high-precision domain regex rules.
- **Shared-Nothing Range Sharding**: Batch throughput scales linearly across CPU cores by sharding the 20-byte `infohash` keyspace (`00..3f`, `40..7f`, `80..bf`, `c0..ff`) with zero inter-process communication.

---

## 2. Directory Layout

```
apps/classifier/
├── README.md                      # Complete system documentation
├── Makefile                       # Developer & runtime commands
├── requirements.txt               # Production Python dependencies
├── requirements-dev.txt           # Test & development dependencies
│
├── src/                           # Core Classifier Package
│   ├── __init__.py
│   ├── classifier_service.py      # Inference service & dual rejection logic
│   ├── feature_extractor.py       # Multi-modal feature pipeline & attribution
│   ├── db.py                      # Read-only PostgreSQL connection pool & queries
│   └── train.py                   # Model training pipeline
│
├── scripts/                       # CLI & Production Operations
│   └── stream_classifier.py       # High-throughput batch streaming worker with range sharding
│
├── web/                           # Web Studio API & UI
│   ├── app.py                     # FastAPI server
│   ├── templates/
│   │   └── index.html             # Minimal Vercel/Linear Dark/Light SPA
│   └── static/                    # Static assets
│
├── models/                        # Serialized Model Artifacts (gitignored)
│   └── torrent_classifier_v2.joblib
│
├── data/                          # Cached Datasets (gitignored)
│   └── labeled_dataset.jsonl
│
└── tools/                         # Auxiliary Tooling
    ├── labeling/                  # Ground truth annotation MCP server & batch tools
    │   ├── mcp_server.py
    │   ├── start_server.sh
    │   ├── extract_batches.py
    │   ├── merge_labeled.py
    │   └── LABELING_GUIDE.md
    └── deepseek/                  # Historical LLM scraping & PoW experiments
```

---

## 3. Quickstart

### Start the Interactive Web Studio
```bash
make serve
# Open http://localhost:8000
```
Features:
- Browse live torrents from PostgreSQL with instant search and quick-filter pills.
- Click any torrent to inspect its complete metadata, full file manifest tree, and real-time classification diagnostics.
- Dark / Light mode toggle (Vercel / Linear inspired aesthetic).

### Retrain Model
```bash
make train
```
Trains on `data/labeled_dataset.jsonl` using calibrated `modified_huber` loss and saves the model to `models/torrent_classifier_v2.joblib`.

### Run Batch Sharded Classification (10M+ Scale)
Run independent worker processes across different hex ranges with zero lock contention:
```bash
# Worker 1 (first quartile)
python scripts/stream_classifier.py --start-hex 00 --end-hex 3f --output-jsonl data/shard_0.jsonl

# Worker 2 (second quartile)
python scripts/stream_classifier.py --start-hex 40 --end-hex 7f --output-jsonl data/shard_1.jsonl

# Worker 3 (third quartile)
python scripts/stream_classifier.py --start-hex 80 --end-hex bf --output-jsonl data/shard_2.jsonl

# Worker 4 (fourth quartile)
python scripts/stream_classifier.py --start-hex c0 --end-hex ff --output-jsonl data/shard_3.jsonl
```

---

## 4. API Reference

### `GET /api/status`
Returns service status, model version, target classes, and database connection state.

### `GET /api/torrents?offset=0&limit=25&search=`
Returns paginated list of torrents from PostgreSQL.

### `GET /api/torrents/{infohash}`
Returns full torrent record including complete files manifest.

### `POST /api/classify`
Classifies a torrent and returns confidence, margins, review flags, full probability distribution, and feature attribution.

**Request body:**
```json
{
  "infohash": "52c4c454a86d57cdef73568c6aa747270c5e816e"
}
```

**Response:**
```json
{
  "torrent": {
    "infohash": "52c4c454a86d57cdef73568c6aa747270c5e816e",
    "name": "Skabma - Snowfall [FitGirl Repack]",
    "total_size": 1694659994,
    "total_size_formatted": "1.58 GB",
    "file_count": 10,
    "files": [...]
  },
  "classification": {
    "predicted_category": "Games",
    "confidence": 1.0,
    "margin": 1.0,
    "needs_review": false,
    "review_reason": null,
    "review_type": "accepted",
    "probabilities": [
      {"category": "Games", "probability": 1.0},
      {"category": "Adult", "probability": 0.0},
      ...
    ],
    "features": {
      "matched_rules": ["Game repack / Console tag (FitGirl/CODEX/PS4/Switch)"],
      "detected_extensions": [".bat", ".bin", ".exe", ".ini", ".md5"]
    }
  }
}
```
