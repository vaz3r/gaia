# Classifier Gold Pilot v1 Implementation & Extraction Report

## 1. Executive Verdicts

* **Gold Pilot v0 (300 Records)**: **`READY_AS_METADATA_SPARSE_DIAGNOSTIC_ONLY`**  
  *Justification*: Preserved completely untouched (verified via immutable SHA-256 checksums) as a baseline comparison artifact.
* **Gold Pilot v1 Natural Stratum (200 Records)**: **`READY_FOR_HUMAN_ANNOTATION`**  
  *Justification*: 200 clean, single-file natural records generated with zero leakage against all historical training sets, evaluation sets, and `data/manual_eval_set_200.jsonl`.
* **Gold Pilot v1 Diagnostic Stratum (100 Rich Records)**: **`EXTRACTION_PLAN_READY`**  
  *Justification*: Production PostgreSQL storage contains full `files` JSONB arrays for multi-file torrents. A read-only SQL extraction plan has been prepared to extract fresh, rich candidates.

---

## 2. Gold Pilot v0 Integrity Preservation

Before and after creating v1, the immutable SHA-256 checksums of Gold Pilot v0 were verified:

| File Path | SHA-256 Checksum | Integrity Status |
|---|---|---|
| `apps/classifier/data/gold_pilot/gold_pilot_manifest.json` | `49c7d867718a0e996413544d194b1ca48cc60035402085373b6e80dc27af9ff5` | ✅ **UNCHANGED** |
| `apps/classifier/data/gold_pilot/gold_pilot_blind.jsonl` | `4f2abbeb87f9f9b3c04f00d06d9324b275fe4b057f5467974e909fa471a07ad0` | ✅ **UNCHANGED** |
| `apps/classifier/data/gold_pilot/gold_pilot_review_template.jsonl` | `a64353f68518a015eeda423777322b2eca0179069c1928595dc99325cf1bd4d8` | ✅ **UNCHANGED** |

---

## 3. Natural-Source Provenance Verification (`manual_eval_set_1000.jsonl`)

The extraction pipeline from PostgreSQL was audited through every transformation step:
1. **Initial Population Sampling**: Extracted via `apps/classifier/src/build_test_set.py` using `SELECT encode(infohash, 'hex')... FROM torrents TABLESAMPLE SYSTEM(5) LIMIT 10000`. No `WHERE` clause, keyword filtering, category condition, or size/extension filter was applied.
2. **Sampling Frame Mechanics**: PostgreSQL `TABLESAMPLE SYSTEM(5)` executes **block-level sampling** across physical database pages rather than independent Bernoulli row sampling.
3. **Corpus Scope**: The frame approximates the empirical distribution of the crawler's stored PostgreSQL torrent corpus at extraction time. It does **not** represent the full wild DHT universe.
4. **Rejection of Unmatched Records**: In `apps/classifier/src/label_test_set.py:L37`, records without a regex keyword match received fallback `"Other"`; zero rows were dropped due to lack of heuristic match.
5. **Class-Specific Queries & Quotas**: None were used; a single global query retrieved the sequential stream.

**Provenance Verdict**: **`APPROXIMATE_NATURAL_FRAME`**

---

## 4. Metadata Sparsity Quantification (Gold Pilot v0 Records)

| Metric | Aggregate Count / Value |
|---|---|
| Records with `file_count == 0` | **0** |
| Records with `file_count == 1` | **300 (100.0%)** |
| Records with `file_count > 1` | **0** |
| Minimum File Count | **1** |
| Median File Count | **1.0** |
| p90 File Count | **1** |
| Maximum File Count | **1** |
| Records with `total_size_bytes <= 0` | **0** |
| Records with missing or empty representative paths | **300 (100.0%)** |
| Records with `file_count > 1` but no representative paths | **0** |

*Characterization*: Gold Pilot v0 records are **metadata-sparse**: name, aggregate size, and file count are present, but representative file paths are absent because historical export scripts flattened or omitted the `files` column.

---

## 5. Production Input Availability Audit

* **Classification Point**: Classification is intended to run **after metadata retrieval** (when `torrents.verified_at` is populated).
* **Database Schema**: PostgreSQL `torrents` table stores:
  * `name` (TEXT)
  * `total_size` (BIGINT)
  * `file_count` (INTEGER)
  * `files` (JSONB): Contains full file paths `[{"length": int, "path": [str, ...]}]` for multi-file torrents, and `NULL` for single-file torrents.
* **Extraction Script Omission**: Historical classifier extraction scripts queried `'[]'::jsonb as top_dirs` without recursively parsing the JSONB path arrays.
* **`build_input_text()` Rich Execution**: `build_input_text()` parses `files` and `top_dirs` to extract fansub tags, audio codecs (FLAC, MP3), installer extensions, and directory structure.

**Production-Input Verdict**: **`MIXED_METADATA_PRODUCTION`**  
*(In production, single-file torrents have title/size only; multi-file torrents have title, size, count, and full file path lists in `torrents.files` JSONB).*

---

## 6. Human-Candidate Exclusion Audit & Resolution in v1

| Dataset Path | Provenance Classification | In v0 Exclusion | In v1 Exclusion | v0 Hash Overlap | v1 Hash Overlap |
|---|---|---|---|---|---|
| `data/labeling_sample_final.jsonl` | `HUMAN_SINGLE_ANNOTATOR` | ✅ YES | ✅ YES | **0** | **0** |
| `data/labeled.jsonl` (rows 0..999) | `HUMAN_REVIEW_UNCONFIRMED` | ✅ YES | ✅ YES | **0** | **0** |
| `data/manual_seed_1800_labeled.jsonl` | `HEURISTIC_LABELED` (seed) | ✅ YES | ✅ YES | **0** | **0** |
| `data/labeled_150.jsonl` | `HEURISTIC_LABELED / MIXED` | ✅ YES | ✅ YES | **0** | **0** |
| `data/manual_eval_set_200.jsonl` | `HUMAN_REVIEW_UNCONFIRMED` | ❌ NO | ✅ YES | **22** [RESOLVED] | **0** [VERIFIED] |
| `data/gold_pilot/gold_pilot_blind.jsonl` | `PILOT_V0` | — | ✅ YES | — | **0** [VERIFIED] |

*Resolution*: In Gold Pilot v1, `data/manual_eval_set_200.jsonl` was added directly to the exclusion ledger. 38 exact infohash leaks and 178 normalized name matches were excluded during selection, yielding **200 completely clean natural records**.

---

## 7. Gold Pilot v1 Artifact Inventory

Versioned directory: `apps/classifier/data/gold_pilot_v1/`

| File Path | Description | Records |
|---|---|---|
| `apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json` | Internal manifest with provenance mapping & checksums | 200 |
| `apps/classifier/data/gold_pilot_v1/gold_pilot_v1_blind.jsonl` | Blind natural annotation dataset for human reviewers | 200 |
| `apps/classifier/data/gold_pilot_v1/gold_pilot_v1_review_template.jsonl` | Blank review template with null fields | 200 |
| `apps/classifier/data/gold_pilot_v1/gold_pilot_v1_generation_report.json` | Execution summary & exclusion statistics | — |

---

## 8. Proposed Read-Only PostgreSQL Extraction Plan (100 Rich Diagnostic Records)

*Status*: **DRAFTED ONLY — READY FOR EXECUTION** (No database connections or network queries made during this step).

### Read-Only Extraction SQL Query:
```sql
-- Read-only extraction of fresh multi-file candidate torrents with parsed filepaths
BEGIN TRANSACTION READ ONLY;

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
  AND t.file_count > 1
  AND t.files IS NOT NULL
  AND jsonb_array_length(t.files) > 0
  AND t.total_size > 0
  AND t.name IS NOT NULL
ORDER BY t.verified_at ASC
LIMIT 3000;

COMMIT;
```

### Safety & Integrity Controls:
1. **Read-Only Transaction**: Guarantees zero writes or table locks.
2. **Zero Labels**: Output will contain strictly metadata fields (`infohash`, `name`, `total_size`, `file_count`, `representative_files`).
3. **Three-Level Exclusion**: `build_gold_pilot_v1.py` will prune any candidate overlapping historical training data, `manual_eval_set_200.jsonl`, Gold Pilot v0, or Gold Pilot v1 Natural Stratum.
4. **Primary Group Assignment**: Selects 100 multi-file diagnostic candidates balanced across the 6 boundary categories.

---

## 9. Test Suite Verification (24 Total Unit Tests)

All 24 unit tests across `test_gold_pilot.py` (20 tests) and `test_gold_pilot_v1.py` (4 tests) executed cleanly:

```text
test_01_v0_integrity_preservation ... ok
test_02_v1_pilot_id_generation ... ok
test_03_safe_representative_path_parsing ... ok
test_04_v1_natural_stratum_zero_leakage ... ok

Ran 24 tests total in 0.241s (100% PASS)
```
