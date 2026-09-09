# Crawler Operations Anomaly Detection (`ml/anomalies`)

Production-grade machine learning anomaly detection system for 24/7 BitTorrent DHT crawler operations. It continuously monitors telemetry counters, routing table states, and verification pipeline latencies to catch operational degradation before it leads to crawler crashes, disk saturation, or network blocks.

---

## 1. System Overview

The system combines:
1. **Unsupervised Multivariate Anomaly Detection:**
   - **Isolation Forest:** Multi-dimensional subspace isolation with feature attribution (deviations against baseline medians in units of IQR).
   - **Autoencoder Reconstruction:** Bottleneck neural network mapping normal diurnal operational cycles; detects unseen failures via reconstruction error.
   - **Statistical Dynamic Thresholds (EWMA & Rolling MAD):** Real-time single-metric confidence envelopes for routing table size and timeout ratios.
2. **Supervised Root-Cause Categorizer:**
   - Multi-class classifier trained on operational incident markers (restarts, slow query logs, DHT routing drops, timeout cascades).
   - Classifies flagged anomalies into actionable incident types:
     - `DB_LATENCY_SPIKE`: Query execution time spikes, thread starvation, autovacuum locks.
     - `DHT_DROP_COLLAPSE`: Routing table collapses, UDP socket dropouts, ISP throttling.
     - `TIMEOUT_CASCADE`: Peer connection timeout surges, seedbox blocking.
     - `RESTART_EVENT`: Crawler process restart or crash recovery.
     - `NORMAL`: Healthy operations.

---

## 2. Directory Layout

```
ml/anomalies/
├── README.md
├── requirements.txt
├── config.py                         # Telemetry definitions, DB settings, and thresholds
├── data/
│   ├── hourly_features.parquet       # 495 historical hourly feature vectors
│   ├── hourly_features.csv           # CSV export for external analysis
│   ├── incident_labels.parquet       # Labeled incident ground truth
│   └── anomaly_backtest_report.png   # 21-day historical evaluation visualization
├── models_storage/
│   ├── isolation_forest.joblib       # Trained Isolation Forest model
│   ├── autoencoder.joblib            # Trained Autoencoder model
│   └── supervised_classifier.joblib  # Trained Supervised Incident Classifier
├── src/
│   ├── api.py                        # FastAPI alerting and real-time inference server
│   ├── data/
│   │   ├── postgres_extractor.py     # Fast extraction of metrics and verification_jobs
│   │   ├── log_extractor.py          # Bounded DuckDB queries to gaia-log-analyzer
│   │   ├── feature_pipeline.py       # Counter reset handling, aggregation, derived ratios
│   │   └── incident_labeler.py       # Incident labeling engine
│   ├── models/
│   │   ├── baseline.py               # EWMA and Median Absolute Deviation baselines
│   │   ├── isolation_forest.py       # Isolation Forest detector
│   │   ├── autoencoder.py            # Autoencoder reconstruction model
│   │   └── supervised.py             # Supervised incident classifier
│   └── pipeline/
│       ├── train.py                  # End-to-end training & evaluation
│       └── detector.py               # Anomaly scoring and root-cause attribution
└── scripts/
    ├── extract_features.py           # CLI to extract and build feature cache
    ├── train_models.py               # CLI to train and save all models
    ├── detect_anomalies.py           # CLI for real-time and historical detection
    └── generate_evaluation_report.py # CLI to generate backtesting visualization plot
```

---

## 3. Quickstart Guide

### 3.1 Setup Environment
```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

### 3.2 Extract Historical Telemetry & Build Dataset
Connects to PostgreSQL (`workspace-production:5432`) and pulls the full telemetry history, computes positive deltas with reset handling, and generates hourly feature vectors:
```bash
python scripts/extract_features.py
```

### 3.3 Train Anomaly Detection Models
Trains the Isolation Forest, Autoencoder, and Supervised Incident Classifier:
```bash
python scripts/train_models.py
```

### 3.4 Run Anomaly Detection CLI

**Analyze the last 24 hours live from Postgres:**
```bash
python scripts/detect_anomalies.py --recent-hours 24
```

**Backtest across the entire 21-day historical cache:**
```bash
python scripts/detect_anomalies.py --from-cache --min-score 0.60
```

### 3.5 Launch Real-Time REST API
```bash
uvicorn src.api:app --host 0.0.0.0 --port 8000 --reload
```
- Health Check: `GET /health`
- Recent Anomalies: `GET /api/anomalies/recent?hours=24&min_score=0.55`

---

## 4. Production Safety Rules
- Telemetry queries on `metrics` use indexed timestamp filters.
- Queries against `gaia-log-analyzer` (DuckDB) must always use bounded date partitions (e.g. `/logs/gaia-node/crawler-YYYY-MM-DD*.jsonl`) rather than unbounded globs (`/logs/**/*.jsonl`) to prevent memory spikes on production.
