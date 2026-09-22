# GAIA Health Scoring Contract

**Algorithm Version:** 2.0.0
**Status:** Milestone 1 (Shadow Mode)
**Last Updated:** September 2026

---

## 1. Ownership Map

| Field | Owner | Writers | Readers |
|---|---|---|---|
| `health_score` | `apps/health-scorer` | Canonical scorer (M2+) | Portal, API, Dashboard, Meilisearch |
| `health_confidence` | `apps/health-scorer` | Canonical scorer (M2+) | Portal, API, Dashboard |
| `health_state` | `apps/health-scorer` | Canonical scorer (M2+) | Portal, API, Dashboard |
| `health_calculated_at` | `apps/health-scorer` | Canonical scorer (M2+) | Portal, API, Dashboard |
| `health_algorithm_version` | `apps/health-scorer` | Canonical scorer (M2+) | Calibration, debugging |
| `last_verified_at` | `apps/health-scorer` | Canonical scorer (M2+) | Portal (freshness display) |
| `last_seed_confirmed_at` | `apps/crawler` | Crawler observations | Scorer (input) |
| `swarm_peers` | `apps/crawler` | Crawler observations | Scorer (input) |
| `swarm_peers_observed_at` | `apps/crawler` | Crawler observations | Scorer (input) |
| `next_health_recalculation_at` | `apps/health-scorer` | Canonical scorer | Scorer scheduler |
| `availability_score` | `apps/health-scorer` | Temporary alias = `health_score` | Legacy consumers (deprecated) |
| `popularity_score` | TBD | Not part of this contract | Portal, API |
| `integrity_score` | `apps/ml/scoring` | ML trust scorer | Dashboard |
| `risk_tier` | `apps/ml/scoring` | ML trust scorer | Portal, API, Meilisearch |
| `policy_action` | `apps/ml/scoring` | ML trust scorer | Portal, API, Meilisearch |

---

## 2. Scoring Formula

### 2.1 Half-Life Decay

```python
decay(age_hours, half_life_hours) = exp(-ln(2) × age_hours / half_life_hours)
```

At `age_hours == half_life_hours`, decay returns exactly 0.5.

### 2.2 Evidence Families

| Family | Weight | Aggregation | Half-life | Bounded By |
|---|---|---|---|---|
| Direct probe/metadata success | 40 | Latest only | 24h | 40 points max |
| Confirmed seed | 30 | Latest only | 36h | 30 points max |
| Peer count | 20 | `min(20, 7 × log2(1 + count))` × decay | 18h | 20 points max |
| DHT sightings | 10 | `min(10, 2 × log2(1 + count_in_12h))` × decay | 12h | 10 points max |
| Probe/metadata failures | −0 to −30 | `min(30, 10 × failures_after_last_success)` × decay | 6h | −30 points max |

### 2.3 Score Computation

```
health_score = clamp(0, 100, round_half_up(
    direct + seed + peer + dht - failure_penalty
))
```

### 2.4 Evidence Normalization

| Raw observation type | Normalized family |
|---|---|
| `metadata_fetch_success` | direct-success |
| `probe_success` | direct-success |
| `metadata_fetch_failure` | direct-failure |
| `probe_failure` | direct-failure |
| `seed_confirmed` | seed |
| `peer_seen` | peer |
| `dht_sighting` | DHT |

### 2.5 Failure Rules

- Only count failures that occurred **after** the latest direct success
- Within the failure lookback window (48h default)
- Capped at 3 before decay is applied
- When no prior success exists: failures reduce confidence but do not produce an overconfident "dead" score of 0

---

## 3. Health State

| State | Condition | Meaning |
|---|---|---|
| `UNKNOWN` | No valid observations | Never probed, no evidence |
| `UNVERIFIED` | Has evidence, no recent direct verification (>72h or only weak evidence) | May be alive but unconfirmed |
| `VERIFIED` | Direct success within 72 hours | Confirmed reachable |
| `STALE` | All evidence older than 7 days | Data exists but may be outdated |

---

## 4. Confidence

`health_confidence` (0.0–1.0) measures **confidence in the health estimate**, NOT likelihood of availability.

```
confidence = min(1.0, 0.6 × freshness + 0.4 × breadth)
```

Where:
- `freshness` = `decay(most_recent_evidence_age, 24h)`
- `breadth` = `families_present / 4`

Minimum confidence floor when all 4 families are present: 0.4 (from breadth component).

---

## 5. Field Semantics

| Field | Type | Description |
|---|---|---|
| `health_score` | SMALLINT 0–100 or NULL | Estimated current availability. NULL = unknown. |
| `health_confidence` | REAL 0.0–1.0 | Confidence in the estimate. 0 = no confidence. |
| `health_state` | VARCHAR(16) | UNKNOWN, UNVERIFIED, VERIFIED, STALE |
| `health_calculated_at` | TIMESTAMPTZ | When the score was last computed |
| `health_algorithm_version` | VARCHAR(16) | Formula version for calibration tracking |
| `last_verified_at` | TIMESTAMPTZ | Latest successful direct verification |
| `last_seed_confirmed_at` | TIMESTAMPTZ | Latest seed confirmation (replaces boolean) |
| `swarm_peers_observed_at` | TIMESTAMPTZ | When swarm_peers was last observed |
| `next_health_recalculation_at` | TIMESTAMPTZ | Scheduled recalculation time (M2+) |

---

## 6. Tiered Recalculation Schedule

| Tier | Rescore interval | Re-verify interval | Conditions |
|---|---|---|---|
| Hot | 15 min | 2–6h | Recent searches, high popularity, near thresholds |
| Active | 1 hour | 12–24h | Any observation in last 7 days |
| Warm | 6 hours | 3–7 days | Seen in last 30 days |
| Cold | 24 hours | No auto-probing | No evidence in 30+ days |

---

## 7. Update Triggers

Score is recalculated within seconds of:
1. A crawler DHT sighting
2. A peer or seed observation
3. A successful or failed metadata/probe outcome

Plus scheduled decay sweeps by tier.

---

## 8. Migration Safety

- No `DROP TABLE` in any migration
- All new constraints added as `NOT VALID` then `VALIDATE CONSTRAINT`
- `CREATE INDEX CONCURRENTLY` for production indexes
- `availability_score` retained as temporary alias during migration
- Dashboard/API made read-only before canonical scorer writes

---

## 9. Calibration

Calibration snapshots record score before probe dispatch. Nightly reports publish per-decile:
- Sample count
- Direct verification success rate
- Average score and score age
- Average confidence
- 95% confidence intervals
- Results by algorithm version

Do not change weights based on low-volume deciles.
