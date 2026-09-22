# Health Scoring Migration — Milestones

**Goal:** Replace three divergent score writers (Rust crawler prober, C# Dashboard API, Python ML scoring worker) with a single canonical health scorer.

**Algorithm:** v2.0.0 — `health_score = clamp(0, 100, direct(40) + seed(30) + peer(20) + dht(10) - failure_penalty(0-30))`

---

## Milestone 1 — Shadow Mode ✅

**Status:** Complete (committed `bbdce37`)

**What it does:**
- Reads production data (SELECT only) from `torrents`, `torrent_availability_observations`, `fetch_peer_outcomes`
- Computes proposed scores using the v2.0.0 formula
- Logs shadow scores as structured JSON (`SHADOW_SCORE` log lines)
- Writes **zero** rows to `health_scores` table
- Validates formula against real production data

**Deliverables:**
- `apps/health-scorer/` — 14 source files, 3 migrations, Dockerfile, 65/65 tests
- Docker compose service in `workspace-production`
- `deploy.sh` integration (data dirs + health check)
- `docs/HEALTH_SCORING_CONTRACT.md`

**Deploy:**
```bash
# 1. Apply migrations (first time only)
./apps/health-scorer/migrations/apply_migration.sh

# 2. Deploy container
./deploy/scripts/deploy.sh workspace-production HEAD health-scorer
```

**Validation:**
- Shadow log entries show proposed scores vs current scores
- Score deltas indicate formula divergence for investigation
- Cursor advances correctly through observation table
- Singleton lock prevents duplicate processing

---

## Milestone 2 — Crawler Integration + Active Scoring

**Status:** Not started

**What it does:**
- Crawler writes real observations to `torrent_availability_observations`
- Health-scorer switches from legacy adapter to real observations
- Health-scorer writes canonical scores to `health_scores` table
- Dashboard reads from `health_scores` instead of `torrents.health_score`
- Feature flag controls the switchover

**Prerequisites:**
- M1 deployed and shadow logs validated
- Score distribution reviewed (no extreme outliers)
- Production backup taken

### M2.1 — Crawler Observation Writes

**Goal:** Crawler inserts rows into `torrent_availability_observations` during existing probe and harvest cycles.

**Changes to `apps/crawler/`:**
- `src/verify/health_prober.rs` — After each probe round, insert observation rows:
  - `metadata_fetch_success` / `metadata_fetch_failure` on probe outcomes
  - `peer_seen` with peer count from DHT sourcing
  - `seed_confirmed` when seeds detected
- `src/harvest/dht.rs` — On DHT sighting, insert `dht_sighting` observation
- New migration: add `observed_at` default to `NOW()` on `torrent_availability_observations`

**Schema:** `torrent_availability_observations` (already exists from M1 migration):
```sql
INSERT INTO torrent_availability_observations
    (infohash, observed_at, observation_type, peer_count, seed_count, source, latency_ms, failure_reason)
VALUES ($1, now(), $2, $3, $4, $5, $6, $7)
```

**Observation types to write:**

| Trigger | observation_type | peer_count | seed_count | source |
|---|---|---|---|---|
| DHT peer response | `peer_seen` | peers_responding | seeds_detected | `crawler` |
| Successful metadata fetch | `metadata_fetch_success` | 0 | 0 | `crawler` |
| Failed metadata fetch | `metadata_fetch_failure` | 0 | 0 | `crawler` |
| Seed detected in swarm | `seed_confirmed` | 0 | 1 | `crawler` |
| DHT lookup sighting | `dht_sighting` | 0 | 0 | `crawler` |

**Performance considerations:**
- Batch inserts (10-50 rows per INSERT UNNEST) to avoid per-row latency
- observations table should handle 100-500 inserts/min at steady state
- Index on `(infohash, observed_at)` supports both scorer reads and pruning

**Testing:**
- Verify observation insert rate matches probe throughput
- Verify cursor advances through new observations
- Shadow mode continues to work alongside real observations

### M2.2 — Health-Scorer: Switch to Real Observations

**Goal:** Health-scorer reads from `torrent_availability_observations` instead of legacy adapter.

**Changes to `apps/health-scorer/`:**
- `scripts/worker.py` — Add `--no-legacy` flag to skip legacy adapter batch
- `src/config.py` — Add `LEGACY_ADAPTER_ENABLED` env var (default `true` for rollout)
- `src/scorer.py` — When observations exist for an infohash, use observation-based evidence; fall back to legacy for torrents with no observations yet

**Dual-source strategy:**
```
For each torrent in batch:
  1. Check if torrent has observations in the table
  2. If yes → use observation_reader evidence (canonical v2.0.0)
  3. If no  → fall back to legacy_adapter evidence
```

**Configuration:**
```bash
LEGACY_ADAPTER_ENABLED=true   # M2.0: dual-source (default)
LEGACY_ADAPTER_ENABLED=false  # M2.1: observations-only
```

**Testing:**
- Run with `LEGACY_ADAPTER_ENABLED=true` — observe both sources in shadow logs
- Compare scores: observation-based vs legacy for same torrents
- Verify no gaps (torrents without observations still scored via legacy)

### M2.3 — Canonical Score Writes

**Goal:** Health-scorer writes computed scores to `health_scores` table.

**Changes to `apps/health-scorer/`:**
- `src/scorer.py` — After computing score, INSERT/UPDATE `health_scores`:
  ```sql
  INSERT INTO health_scores
      (infohash, health_score, health_state, confidence, evidence_summary,
       algorithm_version, health_calculated_at)
  VALUES ($1, $2, $3, $4, $5, '2.0.0', now())
  ON CONFLICT (infohash) DO UPDATE SET
      health_score = EXCLUDED.health_score,
      health_state = EXCLUDED.health_state,
      confidence = EXCLUDED.confidence,
      evidence_summary = EXCLUDED.evidence_summary,
      algorithm_version = EXCLUDED.algorithm_version,
      health_calculated_at = now()
  ```
- `src/config.py` — Add `SHADOW_MODE` env var (default `true`, set to `false` to enable writes)

**Configuration:**
```bash
SHADOW_MODE=true   # M1/M2.0: logging only (default)
SHADOW_MODE=false  # M2.3: writes to health_scores
```

**Safety:**
- Writes only happen when `SHADOW_MODE=false`
- Zero writes until explicitly enabled
- Existing shadow logs validate scores before enabling

**Testing:**
- Verify zero rows in `health_scores` with `SHADOW_MODE=true`
- Enable writes, verify scores appear
- Verify idempotent upserts (re-processing same batch doesn't duplicate)

### M2.4 — Dashboard Read Integration

**Goal:** Dashboard reads from `health_scores` table instead of `torrents.health_score`.

**Changes to `apps/api/`:**
- `Services/DashboardRepository.cs` — LEFT JOIN `health_scores`:
  ```sql
  SELECT t.*, hs.health_score AS canonical_health_score,
         hs.health_state, hs.confidence, hs.health_calculated_at
  FROM torrents t
  LEFT JOIN health_scores hs ON hs.infohash = t.infohash
  ```
- `Endpoints/DashboardTorrentEndpoints.cs` — Return canonical fields when available, fall back to `torrents.health_score`
- `Endpoints/TorrentEndpoints.cs` — Single torrent detail uses canonical score

**Changes to `apps/dashboard/client/`:**
- `App.jsx` — Display `canonical_health_score` when present, fall back to legacy `health_score`
- Color thresholds: same as current (≥70 green, ≥40 amber, <40 red)
- Show health state badge (VERIFIED/UNVERIFIED/STALE/UNKNOWN)
- Show confidence indicator (low confidence = muted color)

**Rollback:**
- Remove LEFT JOIN → back to `torrents.health_score`
- No data loss (health_scores table retains data)

### M2.5 — Consumer Activation

**Goal:** Flip feature flags to make canonical scorer the primary source.

**Feature flags (in `apps/health-scorer/src/config.py`):**
```bash
LEGACY_ADAPTER_ENABLED=false  # Stop reading legacy columns
SHADOW_MODE=false              # Start writing to health_scores
```

**Rollout sequence:**
1. Deploy M2.1 (crawler writes observations) — let observations accumulate 24-48h
2. Deploy M2.2 (dual-source scorer) — shadow mode, validate both sources
3. Deploy M2.3 (enable writes) — set `SHADOW_MODE=false`
4. Deploy M2.4 (dashboard reads) — LEFT JOIN with fallback
5. Deploy M2.5 (disable legacy) — set `LEGACY_ADAPTER_ENABLED=false`

**Each step has a rollback path:**
- Flip env var back → old behavior restored
- No destructive migrations
- health_scores table data preserved

---

## Milestone 3 — Legacy Deprecation

**Status:** Not started (after M2 is stable)

**What it does:**
- Remove old health_score writes from crawler
- Remove legacy adapter from health-scorer
- Clean up deprecated columns
- Remove Dashboard refresh-health endpoint
- Archive shadow logs

### M3.1 — Remove Crawler Health Score Writer

**Changes to `apps/crawler/`:**
- `src/verify/health_prober.rs` — Remove `health_score` and `popularity_score` from UPDATE:
  ```sql
  -- Before:
  UPDATE torrents SET swarm_peers=$2, health_score=$3, popularity_score=$4,
      seed_confirmed=$5, last_health_check=now()
  -- After:
  UPDATE torrents SET swarm_peers=$2, seed_confirmed=$3,
      last_health_check=now()
  ```
- `src/storage/torrents.rs` — Remove default health_score/popularity_score from INSERT
- `src/storage/batch_writer.rs` — Same

**Impact:**
- `torrents.health_score` stops being updated
- `health_scores` table is now the sole source
- Dashboard reads from `health_scores` (M2.4)

### M3.2 — Remove Legacy Adapter

**Changes to `apps/health-scorer/`:**
- Delete `src/legacy_adapter.py`
- `scripts/worker.py` — Remove legacy batch processing path
- `src/scorer.py` — Remove legacy fallback logic
- `src/config.py` — Remove `LEGACY_ADAPTER_ENABLED` env var

**Impact:**
- Only observation-based scoring remains
- All torrents must have observations to be scored
- New torrents without observations get `UNKNOWN` state until first observation

### M3.3 — Schema Cleanup

**New migration:**
```sql
-- Remove deprecated columns from torrents (after confirming no readers)
ALTER TABLE torrents DROP COLUMN IF EXISTS health_score;
ALTER TABLE torrents DROP COLUMN IF EXISTS popularity_score;
ALTER TABLE torrents DROP COLUMN IF EXISTS last_health_check;
ALTER TABLE torrents DROP COLUMN IF EXISTS seed_confirmed;

-- Drop old indexes
DROP INDEX IF EXISTS idx_torrents_health;
DROP INDEX IF EXISTS idx_torrents_popularity;
DROP INDEX IF EXISTS idx_torrents_health_check;
```

**Safety:** Only run after confirming:
- Dashboard reads from `health_scores` (M2.4)
- No other service reads `torrents.health_score`
- ML scoring uses its own `integrity_score` / `risk_tier`

### M3.4 — Remove Dashboard Refresh-Endpoint

**Changes to `apps/api/`:**
- Remove `POST /api/torrents/{infohash}/refresh-health`
- Remove `RefreshTorrentHealthAsync` from `DashboardRepository.cs`
- Remove "Re-check" button from Dashboard UI

**Impact:**
- Health is recalculated by the canonical scorer only
- No on-demand refresh (replaced by scheduled recalculation)

### M3.5 — Archive and Clean Up

- Archive shadow logs for calibration analysis
- Remove `availability_score` alias column from `health_scores`
- Update `HEALTH_SCORING_CONTRACT.md` to reflect final state
- Close issue #1 (crawler migration audit)

---

## Deployment Reference

### Current Production Services

| Service | Writes health_score? | Reads health_score? |
|---|---|---|
| `apps/crawler` (health_prober) | Yes — `torrents.health_score` | N/A |
| `apps/api` (Dashboard) | Yes — on-demand refresh | Yes — serves to frontend |
| `apps/ml/scoring` | No (writes `integrity_score`) | No |
| `apps/health-scorer` | No (M1 shadow mode) | Yes — reads for comparison |

### After M2 Completion

| Service | Writes health_score? | Reads health_score? |
|---|---|---|
| `apps/crawler` | No (M3.1 removes writes) | No |
| `apps/api` | No (M3.4 removes refresh) | Yes — from `health_scores` |
| `apps/ml/scoring` | No | No |
| `apps/health-scorer` | Yes — `health_scores` table | Yes — observations + legacy |

### Rollback Commands

```bash
# Revert to legacy scoring (M2 → M1)
# Set in docker-compose environment:
LEGACY_ADAPTER_ENABLED=true
SHADOW_MODE=true

# Full rollback to pre-M1
# Drop health-scorer container, remove health_scores table
docker compose stop health-scorer
psql -c "DROP TABLE IF EXISTS health_scores, health_scoring_cursor, health_calibration_snapshots;"
```
