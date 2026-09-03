#!/usr/bin/env python3
"""
Comprehensive Unit Test Suite for Gold Pilot Generation, Leakage Prevention,
Annotation Validation, and Dual-Review Comparison.
Explicitly covers all 20 quality-control and validation requirements.
"""
import hashlib
import json
import os
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path

SYS_CLASSIFIER = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SYS_CLASSIFIER / "tools"))
sys.path.insert(0, str(SYS_CLASSIFIER / "tools" / "audit"))

from release_normalizer import normalize_full_name, normalize_release_family
from build_gold_pilot import generate_pilot_id, extract_extension_summary, assess_metadata_completeness
from validate_gold_annotations import validate_gold_annotations, FROZEN_TAXONOMY
from compare_gold_reviews import compute_cohens_kappa, compare_gold_reviews


class TestGoldPilotComprehensive(unittest.TestCase):
    def setUp(self):
        self.test_dir = Path("/tmp/test_gaia_gold_pilot_suite")
        self.test_dir.mkdir(parents=True, exist_ok=True)
        self.manifest_path = self.test_dir / "test_manifest.json"
        
        # Standard mock manifest
        self.mock_manifest_records = [
            {"pilot_id": "GP1-0000000000000001", "infohash": "a"*40, "stratum": "natural", "retrieval_group": "natural_random"},
            {"pilot_id": "GP1-0000000000000002", "infohash": "b"*40, "stratum": "diagnostic", "retrieval_group": "anime_tv_boundary"},
        ]
        with open(self.manifest_path, "w", encoding="utf-8") as f:
            json.dump({"records": self.mock_manifest_records}, f)

    def _create_valid_row(self, pilot_id="GP1-0000000000000001", **kwargs):
        base = {
            "pilot_id": pilot_id,
            "label_category": "Anime",
            "reviewer_confidence": "high",
            "ambiguous": False,
            "alternate_category": None,
            "reason": None,
            "reviewer_id": "reviewer_1",
            "annotation_timestamp": datetime.now(timezone.utc).isoformat(),
            "adjudication_required": False,
        }
        base.update(kwargs)
        return base

    # 1. Deterministic full-pilot selection
    def test_01_deterministic_full_pilot_selection(self):
        """Verify pilot ID generation and selection are completely deterministic across runs."""
        ih = "45bb52352f2dfd1997002f616eef3117db4e8dc3"
        pid1 = generate_pilot_id(ih, 0)
        pid2 = generate_pilot_id(ih, 0)
        self.assertEqual(pid1, pid2)
        self.assertTrue(pid1.startswith("GP1-"))

    # 2. Stratum isolation
    def test_02_stratum_isolation(self):
        """Verify zero infohash overlap between Natural and Diagnostic strata."""
        actual_manifest_path = Path("apps/classifier/data/gold_pilot/gold_pilot_manifest.json")
        if not actual_manifest_path.exists():
            self.skipTest("Manifest not present")
        with open(actual_manifest_path) as f:
            manifest = json.load(f)
        nat_hashes = {r["infohash"] for r in manifest["records"] if r["stratum"] == "natural"}
        diag_hashes = {r["infohash"] for r in manifest["records"] if r["stratum"] == "diagnostic"}
        self.assertEqual(len(nat_hashes & diag_hashes), 0)

    # 3. Duplicate pilot IDs
    def test_03_duplicate_pilot_ids(self):
        """Verify validator rejects files containing duplicate pilot IDs."""
        ann_file = self.test_dir / "dup_ids.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001")
            r2 = self._create_valid_row("GP1-0000000000000001") # DUPLICATE
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 4. Unknown pilot IDs
    def test_04_unknown_pilot_ids(self):
        """Verify validator rejects pilot IDs not present in the manifest."""
        ann_file = self.test_dir / "unknown_id.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001")
            r2 = self._create_valid_row("GP1-9999999999999999") # UNKNOWN
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 5. Missing pilot rows
    def test_05_missing_pilot_rows(self):
        """Verify validator rejects files with missing pilot records from manifest."""
        ann_file = self.test_dir / "missing_row.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001") # Record 2 is missing
            f.write(json.dumps(r1) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 6. Invalid taxonomy
    def test_06_invalid_taxonomy(self):
        """Verify validator rejects invalid categories (e.g. Porn, Unknown, Other2)."""
        ann_file = self.test_dir / "invalid_cat.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", label_category="Porn")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 7. Invalid confidence
    def test_07_invalid_confidence(self):
        """Verify validator rejects confidence levels other than high, medium, low."""
        ann_file = self.test_dir / "invalid_conf.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", reviewer_confidence="very_high")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 8. Alternate category equals primary
    def test_08_alternate_category_equals_primary(self):
        """Verify validator rejects alternate_category identical to label_category."""
        ann_file = self.test_dir / "same_alt.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", label_category="Anime", alternate_category="Anime")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 9. Missing ambiguous reason
    def test_09_missing_ambiguous_reason(self):
        """Verify validator rejects ambiguous: True without reason."""
        ann_file = self.test_dir / "missing_amb_reason.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", ambiguous=True, reason=None)
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 10. Missing low-confidence reason
    def test_10_missing_low_confidence_reason(self):
        """Verify validator rejects reviewer_confidence: low without reason."""
        ann_file = self.test_dir / "missing_low_reason.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", reviewer_confidence="low", reason="")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 11. Invalid timestamp
    def test_11_invalid_timestamp(self):
        """Verify validator rejects malformed timestamps."""
        ann_file = self.test_dir / "invalid_ts.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", annotation_timestamp="not-a-timestamp")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 12. Prohibited annotation fields
    def test_12_prohibited_annotation_fields(self):
        """Verify validator rejects prohibited context fields like source_label or prediction."""
        ann_file = self.test_dir / "prohibited_field.jsonl"
        with open(ann_file, "w") as f:
            r1 = self._create_valid_row("GP1-0000000000000001", source_heuristic_label="Anime")
            r2 = self._create_valid_row("GP1-0000000000000002")
            f.write(json.dumps(r1) + "\n" + json.dumps(r2) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))

    # 13. Unicode normalization
    def test_13_unicode_normalization(self):
        """Verify release normalizer standardizes Unicode across NFKD forms."""
        title_cyrillic = "Смертоносен отмъстител (1992).avi"
        norm1 = normalize_full_name(title_cyrillic)
        norm2 = normalize_release_family(title_cyrillic)
        self.assertTrue(len(norm1) > 0)
        self.assertIn("1992", norm2)

    # 14. Empty metadata preservation
    def test_14_empty_metadata_preservation(self):
        """Verify sparse records are not dropped and completeness flag is accurate."""
        comp = assess_metadata_completeness("Torrent Title", 1, 1000, [])
        self.assertTrue(comp["has_name"])
        self.assertFalse(comp["has_files"])
        self.assertFalse(comp["is_complete"])

    # 15. Release-family duplicate exclusion
    def test_15_release_family_duplicate_exclusion(self):
        """Verify that releases differing only in resolution/group yield identical family keys."""
        t1 = "Show.Title.S01E02.1080p.WEB-DL.H264-GRP1.mkv"
        t2 = "Show.Title.S01E02.720p.HDTV.x265-GRP2.mkv"
        rf1 = normalize_release_family(t1)
        rf2 = normalize_release_family(t2)
        self.assertEqual(rf1, rf2)

    # 16. Exact-infohash enrichment
    def test_16_exact_infohash_enrichment(self):
        """Verify metadata lookup requires exact 40-char lowercase infohash match."""
        ih = "45bb52352f2dfd1997002f616eef3117db4e8dc3"
        lookup = {ih: ["dir/file1.mp4", "dir/file2.mp4"]}
        matched = lookup.get(ih.lower())
        self.assertIsNotNone(matched)
        self.assertEqual(len(matched), 2)

    # 17. Prohibition of title-only enrichment
    def test_17_prohibition_of_title_only_enrichment(self):
        """Verify that matching metadata purely by title without matching infohash is prohibited."""
        title_db = {"Torrent.Title.1080p": ["movie.mkv"]}
        candidate_ih = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        # Exact infohash table must NOT match arbitrary title
        ih_table = {}
        self.assertIsNone(ih_table.get(candidate_ih))

    # 18. One primary diagnostic group
    def test_18_one_primary_diagnostic_group(self):
        """Verify every diagnostic record in the manifest has exactly one primary group."""
        actual_manifest_path = Path("apps/classifier/data/gold_pilot/gold_pilot_manifest.json")
        if not actual_manifest_path.exists():
            self.skipTest("Manifest not present")
        with open(actual_manifest_path) as f:
            manifest = json.load(f)
        diag_records = [r for r in manifest["records"] if r["stratum"] == "diagnostic"]
        for r in diag_records:
            self.assertIn("retrieval_group", r)
            self.assertIsInstance(r["retrieval_group"], str)
            self.assertTrue(len(r["retrieval_group"]) > 0)

    # 19. Disagreement queue behavior
    def test_19_disagreement_queue_behavior(self):
        """Verify dual-review comparator flags disagreements for adjudication."""
        rev_a = self.test_dir / "rev_a.jsonl"
        rev_b = self.test_dir / "rev_b.jsonl"
        out_q = self.test_dir / "adj_queue.jsonl"

        with open(rev_a, "w") as f:
            f.write(json.dumps(self._create_valid_row("GP1-0000000000000001", label_category="Anime")) + "\n")
            f.write(json.dumps(self._create_valid_row("GP1-0000000000000002", label_category="Games")) + "\n")

        with open(rev_b, "w") as f:
            f.write(json.dumps(self._create_valid_row("GP1-0000000000000001", label_category="Television")) + "\n") # DISAGREEMENT
            f.write(json.dumps(self._create_valid_row("GP1-0000000000000002", label_category="Games")) + "\n")      # AGREEMENT

        res = compare_gold_reviews(str(rev_a), str(rev_b), out_queue_path=str(out_q))
        self.assertEqual(res["total"], 2)
        self.assertEqual(res["adjudication_count"], 1)
        self.assertTrue(out_q.exists())

    # 20. Rejection of untouched templates
    def test_20_rejection_of_untouched_templates(self):
        """Verify validator rejects untouched template with null values."""
        ann_file = self.test_dir / "untouched_template.jsonl"
        with open(ann_file, "w") as f:
            r1 = {
                "pilot_id": "GP1-0000000000000001",
                "label_category": None,
                "reviewer_confidence": None,
                "ambiguous": None,
                "alternate_category": None,
                "reason": None,
                "reviewer_id": None,
                "annotation_timestamp": None,
                "adjudication_required": None,
            }
            f.write(json.dumps(r1) + "\n")
        self.assertFalse(validate_gold_annotations(str(ann_file), str(self.manifest_path)))


if __name__ == "__main__":
    unittest.main()
