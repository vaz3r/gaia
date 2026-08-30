# Classifier Gold Pilot Implementation & Provenance Report

## 1. Executive Verdicts

* **Current 300-Record Pilot Verdict**: **`REQUIRES_FRESH_METADATA_EXTRACTION`**  
  *Justification*:
  1. The existing 300 pilot records are **metadata-sparse** (representative file paths are absent), whereas production PostgreSQL storage contains full `files` JSONB arrays for multi-file torrents.
  2. Stratum A contains **22 exact infohash overlaps** with `data/manual_eval_set_200.jsonl` because `manual_eval_set_200.jsonl` was historically categorized as an evaluation set rather than in the training exclusion index.
* **Fresh Rich Pilot Verdict**: **`EXTRACTION_PLAN_READY`**  
  *Justification*: A deterministic, read-only SQL extraction plan has been drafted to pull 150 fresh, rich torrent records (with parsed `files` JSONB paths) free from all historical training, evaluation, and human-candidate overlap.

---

## 2. Natural-Source Provenance Verification (`manual_eval_set_1000.jsonl`)

The extraction pipeline from PostgreSQL was audited through every transformation step:
1. **Initial Population Sampling**: Extracted via `apps/classifier/src/build_test_set.py` using `SELECT encode(infohash, 'hex')... FROM torrents TABLESAMPLE SYSTEM(5) LIMIT 10000`. No `WHERE` clause, keyword filtering, category condition, or size/extension filter was applied.
2. **Sampling Frame Nature**: PostgreSQL `TABLESAMPLE SYSTEM(5)` performs **block-level sampling** on database pages rather than independent Bernoulli row sampling.
3. **Corpus Scope**: The frame approximates the empirical distribution of the crawler's stored PostgreSQL torrent corpus at extraction time. It does **not** claim to represent the entire wild DHT population.
4. **Rejection of Unmatched Records**: In `apps/classifier/src/label_test_set.py:L37`, records without a regex keyword match were assigned fallback `"Other"`. **Zero rows were dropped due to lack of heuristic match.**
5. **Class-Specific Queries**: **None used.** A single global query retrieved the stream.
6. **Altered Distribution / Quotas**: **No per-class quotas, minimum confidence gates, or rejection rules were applied.**

*Provenance Verdict*: **`APPROXIMATE_NATURAL_FRAME`**

---

## 3. Metadata Sparsity Quantification (Current 300 Pilot Records)

All 300 pilot records in `gold_pilot_blind.jsonl` were audited for structural completeness:

| Metric | Aggregate Value | Percentage |
|---|---|---|
| Records with `file_count == 0` | **0** | 0.0% |
| Records with `file_count == 1` | **300** | 100.0% |
| Records with `file_count > 1` | **0** | 0.0% |
| Minimum File Count | **1** | — |
| Median File Count | **1.0** | — |
| p90 File Count | **1** | — |
| Maximum File Count | **1** | — |
| Records with `total_size_bytes <= 0` | **0** | 0.0% |
| Records with missing or empty representative paths | **300** | 100.0% |
| Records with `file_count > 1` but no representative paths | **0** | 0.0% |

*Finding*: The pilot records are **metadata-sparse**: name, aggregate size, and file count are present, but representative file paths are absent because source evaluation files (`manual_eval_set_1000.jsonl` and `manual_eval_set_balanced_2000.jsonl`) flattened or omitted the `files` column during historical export.

---

## 4. Production Input Availability Audit

Audit of `apps/crawler/src/storage/torrents.rs`, `apps/crawler/migrations/0001_init.sql`, and `apps/classifier/src/core/text_builder.py`:
1. **Classification Timing**: Classification is designed to execute **after metadata retrieval** (when `torrents.verified_at` is set by the crawler).
2. **Database Storage**: The PostgreSQL `torrents` table contains:
   * `name` (TEXT)
   * `total_size` (BIGINT)
   * `file_count` (INTEGER)
   * `files` (JSONB): Contains full file paths `[{"length": int, "path": [string, ...]}]` for multi-file torrents, and `NULL` for single-file torrents.
3. **Extraction Script Omission**: Historical extraction scripts (`src/build_balanced_eval_set.py`, `src/extract_large_pool.py`) queried `'[]'::jsonb as top_dirs` or extracted `f->>'path'` without recursively parsing the JSONB path arrays.
4. **`build_input_text()` Rich Execution**: `build_input_text()` explicitly parses `files` and `top_dirs` to detect fansub releases, audio formats (FLAC, MP3), executable installers, and media file hierarchies.

*Production-Input Verdict*: **`MIXED_METADATA_PRODUCTION`**  
*(In production, single-file torrents have title/size only; multi-file torrents have title, size, count, and full file path lists stored in `torrents.files` JSONB).*

---

## 5. Diagnostic Allocation Audit (100 Stratum B Records)

| Primary Retrieval Group | Target Quota | Actual Collected | Quota Shortfall | Source Datasets |
|---|---|---|---|---|
| `anime_tv_boundary` | 20 | **20** | 0 | `eval_balanced_2000` (14), `al_1129` (6) |
| `app_game_boundary` | 20 | **20** | 0 | `eval_balanced_2000` (12), `al_1129` (8) |
| `game_other_boundary` | 15 | **15** | 0 | `eval_balanced_2000` (11), `edge_cases_large` (4) |
| `movie_doc_boundary` | 15 | **15** | 0 | `eval_balanced_2000` (10), `al_1129` (5) |
| `rare_music_doc` | 15 | **15** | 0 | `eval_balanced_2000` (10), `al_1129` (5) |
| `difficult_other_mixed` | 15 | **15** | 0 | `eval_balanced_2000` (4), `al_1129` (2), `edge_cases_large` (7), `edge_cases` (2) |
| **Total** | **100** | **100** | **0** | **100.0% Quota Met** |

* **Multi-Group Qualification**: **56 of 100 diagnostic records** matched keywords for more than one group.
* **Assignment Priority**: Assigned sequentially to the first unsaturated group in priority order: `anime_tv` $\to$ `app_game` $\to$ `game_other` $\to$ `movie_doc` $\to$ `rare_music_doc` $\to$ `difficult_other_mixed`.
* **Deduplication Timing**: Strict deduplication against training data and Stratum A was enforced **before** group allocation.
* **Primary Group Exclusivity**: Every diagnostic record has **exactly one primary retrieval group** recorded in `gold_pilot_manifest.json`. Retrieval metadata is completely withheld from blind and review files.

---

## 6. Human-Candidate Exclusion Audit

All historical human-annotated and human-review candidate datasets were audited:

| Dataset Path | Current Provenance | SHA-256 | In Exclusion List | Hash Overlap with Pilot | Name Overlap | Family Overlap |
|---|---|---|---|---|---|---|
| `apps/classifier/data/labeling_sample_final.jsonl` | `HUMAN_SINGLE_ANNOTATOR` | `2bf217afdb...` | ✅ YES | **0** | **0** | **0** |
| `apps/classifier/data/labeled.jsonl` (rows 0..999) | `HUMAN_REVIEW_UNCONFIRMED` | `1130978bf3...` | ✅ YES | **0** | **0** | **0** |
| `apps/classifier/data/manual_seed_1800_labeled.jsonl` | `HEURISTIC_LABELED` (seed) | `93e5dd4f65...` | ✅ YES | **0** | **0** | **0** |
| `apps/classifier/data/manual_seed_1800.csv` | `HEURISTIC_LABELED` (seed csv)| `8f476a867c...` | ❌ NO | **0** | **0** | **0** |
| `apps/classifier/data/labeled_150.jsonl` | `HEURISTIC_LABELED / MIXED` | `37797091e4...` | ✅ YES | **0** | **0** | **0** |
| `apps/classifier/data/edge_cases.jsonl` | `HEURISTIC / DIAGNOSTIC` | `f084a1cd3f...` | ❌ NO (Source) | **2** (Stratum B source) | **2** | **2** |
| `apps/classifier/data/manual_eval_set_200.jsonl` | `HUMAN_REVIEW_UNCONFIRMED` | `1b9588b0b7...` | ❌ NO | **22** [FAIL] | **22** | **22** |

#### Exclusion Failure Finding
* `manual_eval_set_200.jsonl` was omitted from `TRAINING_DATASETS` in `build_gold_pilot.py` because it was classified as an evaluation set from Iteration 2 rather than a training set.
* Consequently, **22 records in Stratum A** share exact infohashes with `manual_eval_set_200.jsonl`.
* **Remediation**: `manual_eval_set_200.jsonl` must be added to the exclusion index. 22 clean replacement natural records will be drawn from the remaining 777 leak-free candidates in `manual_eval_set_1000.jsonl` upon next authorized build.

---

## 7. Expanded Test Suite Coverage (20 Cases)

`apps/classifier/tests/test_gold_pilot.py` was executed with all 20 explicit requirements passing:

```text
test_01_deterministic_full_pilot_selection (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_02_stratum_isolation (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_03_duplicate_pilot_ids (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_04_unknown_pilot_ids (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_05_missing_pilot_rows (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_06_invalid_taxonomy (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_07_invalid_confidence (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_08_alternate_category_equals_primary (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_09_missing_ambiguous_reason (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_10_missing_low_confidence_reason (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_11_invalid_timestamp (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_12_prohibited_annotation_fields (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_13_unicode_normalization (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_14_empty_metadata_preservation (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_15_release_family_duplicate_exclusion (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_16_exact_infohash_enrichment (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_17_prohibition_of_title_only_enrichment (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_18_one_primary_diagnostic_group (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_19_disagreement_queue_behavior (test_gold_pilot.TestGoldPilotComprehensive) ... ok
test_20_rejection_of_untouched_templates (test_gold_pilot.TestGoldPilotComprehensive) ... ok

----------------------------------------------------------------------
Ran 20 tests in 0.046s

OK
```

---

## 8. Proposed Read-Only Rich Extraction Plan (150 Candidates)

*Status*: **DRAFTED ONLY — NOT EXECUTED** (No database connections or network queries made).

### Proposed SQL Query Template:
```sql
-- Read-only extraction of 150 rich torrent records with parsed JSONB filepaths
WITH source_pool AS (
    SELECT 
        encode(t.infohash, 'hex') AS infohash,
        t.name,
        t.total_size,
        t.file_count,
        COALESCE(
            (
                SELECT jsonb_agg(
                    CASE 
                        WHEN jsonb_typeof(f->'path') = 'array' THEN 
                            (SELECT string_agg(elem::text, '/') FROM jsonb_array_elements_text(f->'path') AS elem)
                        ELSE f->>'path'
                    END
                )
                FROM jsonb_array_elements(t.files) AS f
            ),
            '[]'::jsonb
        ) AS representative_files,
        t.verified_at
    FROM torrents t
    WHERE t.verified_at >= '2026-08-01 00:00:00+00' 
      AND t.verified_at < '2026-08-25 00:00:00+00'
      AND t.name IS NOT NULL
      AND octet_length(t.infohash) = 20
)
SELECT * 
FROM source_pool
ORDER BY verified_at ASC
LIMIT 300;
```

### Extraction Quality Controls:
1. **Zero Labels**: Output will contain strictly metadata fields (`infohash`, `name`, `total_size`, `file_count`, `representative_files`).
2. **Three-Level Leakage Pruning**: Python post-processor will exclude any row whose infohash, normalized name, or release-family key exists in the comprehensive exclusion ledger (including `manual_eval_set_200.jsonl`).
3. **Deterministic Sampling**: The first 100 leak-free natural records and 50 leak-free diagnostic records will be selected.
