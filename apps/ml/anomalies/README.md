# Gaia Crawler Operations Anomaly Detection (`apps/ml/anomalies`)

Production-grade machine learning anomaly detection system for 24/7 BitTorrent DHT crawler operations. It continuously monitors telemetry counters, routing table states, transport outcomes, and internal pipeline queues to catch operational degradation before it leads to crawler crashes, disk saturation, or network blocks.

Currently deployed and running live as the `gaia-anomaly-worker` daemon under `deploy/targets/workspace-production/docker-compose.yml`.

---

## 1. What Problems Does This System Solve?

The DHT crawler runs 24/7, processing millions of infohashes and handling hundreds of thousands of peer connections. Most network, transport, or database degradations do not produce immediate fatal errors; instead, they silently degrade throughput or saturate queues until the process crashes or gets blocked.

The anomaly detector monitors **5 critical operational failure modes**:

### 1. Silent IP / Seedbox / ISP Blocking (`TIMEOUT_CASCADE`)
- **The Failure:** Seedbox swarms or transit ISPs (e.g. OVH) may silently throttle or block the crawler's IP on UDP or peer wire ports. The crawler continues firing millions of requests, but virtually all time out. Verified torrents per hour plunge to near zero.
- **Monitored Telemetry:** `fetch_connect_timeout_ratio`, `source_timeout_ratio`, `tcp_success_ratio`, `utp_success_ratio`, and asymmetry in `tcp_to_utp_ratio`.
- **Alert & Action:** Flags `TIMEOUT_CASCADE`, indicating connection timeouts surged past statistical baselines. Action: inspect IP reputation, test direct peer wire connectivity, or rotate IP.

### 2. Routing Table Collapses & DHT Throttling (`DHT_DROP_COLLAPSE`)
- **The Failure:** If the crawler's UDP socket drops packets, bootstrap nodes stall, or external networks rate-limit inbound traffic, the DHT routing table can collapse from ~13,000 healthy nodes down to a few hundred. Inbound discovery (`get_peers`) drops precipitously.
- **Monitored Telemetry:** First-order derivative of routing table size ($\Delta \text{nodes}/\Delta t$), `inbound_get_peers_rate`, `inbound_find_node_rate`, and `inbound_dropped_rate_limit_ratio`.
- **Alert & Action:** Flags `DHT_DROP_COLLAPSE`. Action: verify UDP socket binding, inspect ISP UDP firewall rules, and check connectivity to DHT bootstrap nodes.

### 3. Queue Saturation & Pre-Crash Memory Leaks (`QUEUE_SATURATION`)
- **The Failure:** If downstream verification stalls, internal channel queues (`fresh_channel_depth`, `verify_channel_depth`) climb towards their $65,536$ capacity. Once full, millions of items are dropped, the scheduler blocks, and the process risks OOM crashing or deadlocking. *(Historical backtesting confirmed this occurred on Aug 26 and Aug 29!)*
- **Monitored Telemetry:** Rolling mean channel depth, `fresh_channel_dropped_rate`, and `scheduler_send_blocked_rate`.
- **Alert & Action:** The Autoencoder and Isolation Forest detect queue accumulation *hours before* drops occur, warning you to relieve downstream bottlenecks or clear slow DB queries.

### 4. Database Latency Spikes & Lock Contention (`DB_LATENCY_SPIKE`)
- **The Failure:** Heavy batch writes or autovacuum lock contention on `torrents` (2.7M rows) or `verification_jobs` (6.5M rows) can block crawler worker threads on database I/O.
- **Monitored Telemetry:** Correlation between `scheduler_send_blocked_rate` and slow query events (`elapsed_secs > 5s`) logged in `gaia-log-analyzer`.
- **Alert & Action:** Flags `DB_LATENCY_SPIKE`. Action: inspect PostgreSQL locks, slow query statements, and autovacuum status.

### 5. Crash Loops & Restart Storms (`RESTART_EVENT`)
- **The Failure:** If the crawler repeatedly crashes and restarts (e.g. killed by the Linux OOM killer), peer caches and DHT state are repeatedly wiped while sighting logs appear deceptively active.
- **Monitored Telemetry:** `_session_start` markers and negative counter discontinuities.
- **Alert & Action:** Flags `RESTART_EVENT`. Action: inspect systemd journal, host memory, and dmesg for kernel OOM kills.

---

## 2. Production Worker Architecture (`gaia-anomaly-worker`)

The system is deployed as a long-running daemon container operating on two decoupled cycles:

```
                                  ┌─────────────────────────────────────────────────────────────┐
                                  │                gaia-anomaly-worker                          │
                                  │           (docker: network_mode: host)                      │
                                  └──────────────────────────────┬──────────────────────────────┘
                                                                 │
                                ┌────────────────────────────────┴────────────────────────────────┐
                                │                                                                 │
                                ▼                                                                 ▼
                [ Detection Loop: Every 15 Minutes ]                              [ Retraining Loop: Every 24 Hours ]
                1. Query last 3h telemetry from PostgreSQL                       1. Extract 14–30 days historical telemetry
                2. Calculate sliding window rates & ratios                       2. Extract restart & slow-query incident markers
                3. Compute ensemble anomaly score (IF + AE)                      3. Retrain Isolation Forest, Autoencoder & RF
                4. Match supervised incident categories                          4. Validate model convergence & sanity checks
                5. If score >= 0.60:                                             5. Atomically update models on disk
                   - Record in DB (`operational_alerts`)                         6. Hot-reload models into memory without downtime
                   - Optional webhook notification
```

### Worker Features:
- **15-Minute Detection Loop:** Queries the indexed `metrics` table for recent snapshots ($< 50\text{ms}$ query time), calculates positive deltas with reset handling, and scores recent windows.
- **24-Hour Automated Retraining:** Re-extracts recent operational history, retrains all three models to adapt to organic DHT seasonal shifts, saves weights to `/app/models_storage`, and calls `detector.reload_models()` for **zero-downtime hot-reloading**.
- **Alert Deduplication:** Employs a 30-minute cool-off window per incident type to eliminate redundant notifications during sustained outages.
- **Healthcheck & Resiliency:** Writes timestamp heartbeats to `/tmp/worker_heartbeat` validated by Docker healthcheck (`healthy`). Gracefully intercepts `SIGTERM` and `SIGINT`.

---

## 3. Database Alert Persistence (`operational_alerts`)

When an anomaly is detected, a structured record is written to the `operational_alerts` PostgreSQL table (migration `0010_operational_alerts.sql`):

### Table Schema:
```sql
CREATE TABLE IF NOT EXISTS operational_alerts (
    id               BIGSERIAL PRIMARY KEY,
    ts               TIMESTAMPTZ NOT NULL DEFAULT now(),
    anomaly_score    DOUBLE PRECISION NOT NULL,
    severity         TEXT NOT NULL,             -- 'INFO', 'WARNING', 'CRITICAL'
    incident_type    TEXT NOT NULL,             -- 'DB_LATENCY_SPIKE', 'DHT_DROP_COLLAPSE', etc.
    confidence       DOUBLE PRECISION,
    top_features     JSONB,                     -- Feature values, baselines, and deviations
    guidance         TEXT,                      -- Actionable mitigation advice
    resolved_at      TIMESTAMPTZ
);
```

### Example Alert Record:
```json
{
  "id": 42,
  "ts": "2026-08-26T17:00:00Z",
  "anomaly_score": 0.825,
  "severity": "WARNING",
  "incident_type": "RESTART_EVENT",
  "confidence": 0.89,
  "top_features": [
    {
      "feature": "fresh_channel_dropped_rate",
      "current_value": 1040026.0,
      "baseline_median": 0.0,
      "z_deviation": 1040026.0
    },
    {
      "feature": "verify_channel_depth_mean",
      "current_value": 56961.2,
      "baseline_median": 0.0,
      "z_deviation": 56961.2
    }
  ],
  "guidance": "Crawler restart or process crash event detected. Verify host memory limits, OOM logs, and systemd journal."
}
```

---

## 4. Multi-Modal Telemetry & Models

### 4.1 Telemetry Sources
1. **PostgreSQL Telemetry (`metrics`):** 1.77M rows spanning 21 continuous days (August 19 – September 9, 2026), sampled at 60s intervals. Covers ~60 metrics across DHT inbound/outbound, routing table, verification pipelines, peer sourcing, and transport protocols.
2. **`gaia-log-analyzer` & DuckDB Logs:** 8.1 GB of rotated JSONL traces (`/logs/gaia-node/`). Emits exact `"slow statement"` database latency events and `"connect_failed_sample"` errors. *Safety rule: Always queries partition-bounded globs to prevent memory spikes on production.*

### 4.2 Machine Learning Models
1. **Isolation Forest (`src/models/isolation_forest.py`):** Unsupervised multivariate subspace isolation with 150 estimators and robust scaling. Provides feature attribution by computing deviation from median in units of IQR.
2. **Autoencoder (`src/models/autoencoder.py`):** Non-linear neural bottleneck architecture $(16 \to 8 \to 16)$ learning normal diurnal crawler cycles. Detects unseen failure modes via high reconstruction MSE.
3. **Supervised Incident Classifier (`src/models/supervised.py`):** Random Forest classifier mapping multivariate anomalies to actionable root-cause categories (`NORMAL`, `DB_LATENCY_SPIKE`, `DHT_DROP_COLLAPSE`, `TIMEOUT_CASCADE`, `RESTART_EVENT`).
4. **Statistical Baselines (`src/models/baseline.py`):** Rolling EWMA and Modified Z-Score (MAD) for instant single-metric threshold violations.

---

## 5. Directory Layout

```
apps/ml/anomalies/
├── README.md                         # Operator and system documentation
├── requirements.txt                  # Pinned dependencies (scikit-learn, pyarrow, etc.)
├── Dockerfile                        # Production container image definition
├── config.py                         # Telemetry definitions, DB settings, and thresholds
├── migrations/
│   └── 0001_operational_alerts.sql  # Database schema migration
├── data/
│   ├── hourly_features.parquet       # Historical hourly feature vectors
│   ├── hourly_features.csv           # CSV export for analysis
│   ├── incident_labels.parquet       # Labeled incident ground truth
│   └── anomaly_backtest_report.png   # 21-day historical evaluation visualization
├── models_storage/
│   ├── isolation_forest.joblib       # Trained Isolation Forest model
│   ├── autoencoder.joblib            # Trained Autoencoder model
│   └── supervised_classifier.joblib  # Trained Supervised Incident Classifier
├── src/
│   ├── api.py                        # FastAPI alerting and real-time inference server
│   ├── worker.py                     # Production daemon (15m detection, 24h retraining)
│   ├── data/
│   │   ├── alert_recorder.py         # Postgres alert persistence & webhook client
│   │   ├── feature_pipeline.py       # Monotonic counter deltas, resets, ratios
│   │   ├── incident_labeler.py       # Automated incident labeling engine
│   │   ├── log_extractor.py          # Bounded DuckDB queries to gaia-log-analyzer
│   │   └── postgres_extractor.py     # SQL telemetry extractor
│   ├── models/
│   │   ├── autoencoder.py            # Reconstruction-based neural Autoencoder
│   │   ├── baseline.py               # EWMA and Median Absolute Deviation baselines
│   │   ├── isolation_forest.py       # Isolation Forest detector
│   │   └── supervised.py             # Supervised incident classifier
│   └── pipeline/
│       ├── detector.py               # Anomaly scoring and root-cause attribution
│       └── train.py                  # End-to-end model training and artifact saving
└── scripts/
    ├── detect_anomalies.py           # CLI for live and historical detection
    ├── extract_features.py           # CLI to extract and build feature cache
    ├── generate_evaluation_report.py # Visual backtest report generator
    └── train_models.py               # CLI to train and save all models
```

---

## 6. Operator Runbook & Commands

### 6.1 Inspecting Live Container on Production
```bash
# Check worker status
docker ps --filter "name=gaia-anomaly-worker"

# Inspect live detection cycle logs
docker logs --tail 50 -f gaia-anomaly-worker
```

### 6.2 Querying Active Alerts from PostgreSQL
```sql
SELECT ts, severity, incident_type, anomaly_score, guidance, top_features
FROM operational_alerts
ORDER BY ts DESC
LIMIT 10;
```

### 6.3 Local CLI Usage

**Run single detection cycle on recent hours:**
```bash
python scripts/detect_anomalies.py --recent-hours 24
```

**Backtest across historical cache:**
```bash
python scripts/detect_anomalies.py --from-cache --min-score 0.60
```

**Force immediate retraining:**
```bash
python src/worker.py --retrain-now
```

**Run local worker single-pass test:**
```bash
python src/worker.py --run-once
```

### 6.4 Deploying Updates
To deploy updates to `workspace-production` without touching other running containers:
```bash
./deploy/scripts/deploy.sh workspace-production HEAD anomaly-worker
```
*(Uses `--no-deps` and `--no-recreate` to ensure zero disturbance to postgres, dashboard, or other production services.)*
