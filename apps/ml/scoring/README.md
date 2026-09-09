# Gaia Torrent Quality, Trust & Availability Scoring (`apps/ml/scoring`)

A production-grade, dual-stage machine learning system that scores torrent safety, structural organization, and real-time swarm availability across millions of crawled torrents.

---

## 1. Architectural Principles & Orthogonality

Rather than combining safety, structure, and peer health into a single misleading score, the system separates them into three independent dimensions:

1. **Integrity & Safety (`integrity_score`: $0\text{--}100$, `risk_tier`):**
   - Answers: *Is this payload authentic, non-deceptive, and safe?*
   - Combines a calibrated machine learning probability ($P_{\text{safe}} \in [0.0, 1.0]$) with deterministic safety deductions and critical invariants.
   - **Tiers:** `SAFE` ($\ge 80$), `REVIEW` ($50\text{--}79$), `SUSPICIOUS` ($20\text{--}49$), `BLOCKED` ($< 20$).
   - **Policy Actions:** `ALLOW` ($\ge 70$), `DOWNRANK` ($40\text{--}69$), `REVIEW` ($< 40$ or uncertain), `SUPPRESS` (critical invariants or confirmed fakes). Low score alone never blocks; `SUPPRESS` is strictly reserved for deterministic invariants or high-precision thresholds.
2. **Metadata Quality (`metadata_quality_score`: $0\text{--}100$):**
   - Answers: *Is this torrent cleanly named and structured for search?*
   - Deterministic policy deduction ($100 - \sum \text{penalties}$) measuring title structure, primary payload dominance ($> 60\%$), path depth hygiene ($\le 6$), and junk file ratio ($< 35\%$).
3. **Availability (`availability_score`: $0\text{--}100$, `availability_state`):**
   - Answers: *Can peers be reached to download this content right now?*
   - Time-decayed operational score tracking confirmed seeds ($+50$), active probed peer count ($\le 30$), and sighting recency ($20 \cdot e^{-\Delta t / 7\text{d}}$).
   - **States:** `ACTIVE` ($\ge 60$), `DEGRADED` ($25\text{--}59$), `STALE` ($1\text{--}24$), `UNKNOWN` ($0$).
   - **Core Invariant:** An authentic 10-year-old Linux ISO with 0 peers has `integrity_score = 100`, `risk_tier = SAFE`, and `availability_state = STALE`. It is never labeled as spam.

---

## 2. Models & Components

### 2.1 Model 1: PreVerifier Prioritizer
Ranks pending infohashes in the crawler verification queue by **Expected Utility**:
$$\text{Priority Utility} = \frac{P(\text{verified\_15m}) \times \text{Catalog Value}}{\max(\text{Expected Cost Seconds}, \text{Cost Floor})} + \text{Explore Bonus} + \text{Long-tail Bonus} - \text{Retry Penalty}$$

- **Outcome A Head:** Predicts $P(\text{verified\_within\_15m\_budget})$.
- **Outcome B Head:** Predicts $P(\text{verified\_within\_72h\_survival})$.
- **Exploration Traffic:** $10\%$ randomized exploration traffic reserved to prevent self-reinforcing queue bias.
- **Zero Leakage:** Uses point-in-time sighting velocities and announce-to-get_peers ratios strictly as known at prediction timestamp $t$.

### 2.2 Model 2: Calibrated TrustClassifier
- **Architecture:** Histogram-based Gradient Boosting Classifier with Isotonic probability calibration.
- **Validation Split:** 70% Train (oldest), 15% Calibration (middle), 15% Holdout Test (newest), disjoint by infohash.
- **Validation Results:**
  - **Accuracy:** $99.85\%$
  - **Brier Score:** $0.0014$
  - **Expected Calibration Error (ECE):** $0.0006$ (Target $< 0.05$)
  - **Suppression Precision:** $99.39\%$ (Target $\ge 99.5\%$)
  - **False Suppression Rate:** $0.037\%$ (Target $< 0.5\%$)
  - **Gold Malicious Recall:** $98.7\%$ ($148/150$)
  - **Gold Clean Retention:** $100.0\%$ ($150/150$)

---

## 3. Database Schema & Stored Fields

Summary columns on `torrents`:
```sql
integrity_score        SMALLINT CHECK (integrity_score BETWEEN 0 AND 100),
model_safe_probability REAL CHECK (model_safe_probability BETWEEN 0.0 AND 1.0),
policy_integrity_score SMALLINT CHECK (policy_integrity_score BETWEEN 0 AND 100),
policy_action          VARCHAR(16) CHECK (policy_action IN ('ALLOW', 'DOWNRANK', 'REVIEW', 'SUPPRESS')),
policy_version         VARCHAR(64),
decision_source        VARCHAR(16) CHECK (decision_source IN ('MODEL', 'POLICY', 'MODEL_AND_POLICY', 'MANUAL')),
metadata_quality_score SMALLINT CHECK (metadata_quality_score BETWEEN 0 AND 100),
availability_score     SMALLINT CHECK (availability_score BETWEEN 0 AND 100),
risk_tier              VARCHAR(16) CHECK (risk_tier IN ('SAFE', 'REVIEW', 'SUSPICIOUS', 'BLOCKED')),
availability_state     VARCHAR(16) CHECK (availability_state IN ('ACTIVE', 'DEGRADED', 'UNKNOWN', 'STALE')),
score_model_version    VARCHAR(64),
scored_at              TIMESTAMPTZ
```

Audit tables:
- `torrent_score_history`: Preserves every scoring event, reason codes, model version, and run ID.
- `torrent_availability_observations`: Separates transient swarm probes from stable metadata scores.
- `scoring_checkpoints`: Tracks keyset pagination checkpoints for resumable backfills.

---

## 4. Runbooks & Operational Commands

### Run Unit & Invariant Tests
```bash
python -m tests.run_tests
```

### Run Dataset Audit
```bash
python scripts/audit_dataset.py
```

### Retrain Model & Generate Calibration Report
```bash
python src/pipeline/train.py
```

### Run Keyset Backfill Engine
```bash
# Backfill next N batches (2,000 items each)
python scripts/backfill_scores.py 10

# Continuous backfill of entire catalog
python scripts/backfill_scores.py
```

### Run Daemon Worker (Shadow Mode)
```bash
python src/worker.py
```
