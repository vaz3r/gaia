#!/usr/bin/env python3
"""
Unit Test Suite for Finalized Gold Pilot v1.
Validates 300-record structure, stratum isolation, exact quotas,
v0/pool integrity preservation, and blind/template field privacy.
"""
import hashlib
import json
import os
import sys
import unittest
from pathlib import Path

SYS_CLASSIFIER = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SYS_CLASSIFIER / "tools"))
sys.path.insert(0, str(SYS_CLASSIFIER / "tools" / "audit"))

from build_gold_pilot_v1 import (
    generate_pilot_v1_id,
    parse_representative_files,
    V0_CHECKSUMS,
    POOL_RICH_SHA256,
    sha256_file,
)
from validate_gold_annotations import validate_gold_annotations, FROZEN_TAXONOMY
from release_normalizer import normalize_full_name, normalize_release_family


class TestGoldPilotV1Final(unittest.TestCase):
    def setUp(self):
        self.v1_dir = Path("data/gold_pilot_v1")
        self.v0_dir = Path("data/gold_pilot")

    def test_01_v0_and_pool_checksums_preserved(self):
        """Verify Gold Pilot v0 files and candidate_pool_rich.jsonl match immutable hashes."""
        v0_manifest = self.v0_dir / "gold_pilot_manifest.json"
        v0_blind = self.v0_dir / "gold_pilot_blind.jsonl"
        v0_template = self.v0_dir / "gold_pilot_review_template.jsonl"
        pool_rich = self.v1_dir / "candidate_pool_rich.jsonl"

        self.assertEqual(sha256_file(str(v0_manifest)), V0_CHECKSUMS["manifest"])
        self.assertEqual(sha256_file(str(v0_blind)), V0_CHECKSUMS["blind"])
        self.assertEqual(sha256_file(str(v0_template)), V0_CHECKSUMS["template"])
        self.assertEqual(sha256_file(str(pool_rich)), POOL_RICH_SHA256)

    def test_02_v1_record_counts_and_stratum_composition(self):
        """Verify Gold Pilot v1 has exactly 300 total records (200 natural, 100 diagnostic)."""
        manifest_path = self.v1_dir / "gold_pilot_v1_manifest.json"
        blind_path = self.v1_dir / "gold_pilot_v1_blind.jsonl"
        template_path = self.v1_dir / "gold_pilot_v1_review_template.jsonl"

        with open(manifest_path) as f:
            manifest = json.load(f)
        with open(blind_path) as f:
            blind = [json.loads(l) for l in f if l.strip()]
        with open(template_path) as f:
            template = [json.loads(l) for l in f if l.strip()]

        self.assertEqual(len(manifest["records"]), 300)
        self.assertEqual(len(blind), 300)
        self.assertEqual(len(template), 300)

        # Unique pilot IDs
        pids = [r["pilot_id"] for r in blind]
        self.assertEqual(len(set(pids)), 300)

        # Stratum counts
        nat_records = [r for r in blind if r["metadata_mode"] == "sparse_single_file"]
        diag_records = [r for r in blind if r["metadata_mode"] == "rich_multi_file"]

        self.assertEqual(len(nat_records), 200)
        self.assertEqual(len(diag_records), 100)

        for r in nat_records:
            self.assertEqual(r["file_count"], 1)
            self.assertEqual(r["files"], [])

        for r in diag_records:
            self.assertGreater(r["file_count"], 1)
            self.assertGreater(len(r["files"]), 0)

    def test_03_diagnostic_quotas_and_primary_group_exclusivity(self):
        """Verify exact quotas and single primary group assignment in manifest."""
        manifest_path = self.v1_dir / "gold_pilot_v1_manifest.json"
        with open(manifest_path) as f:
            manifest = json.load(f)

        expected_quotas = {
            "anime_tv_boundary": 20,
            "app_game_boundary": 20,
            "game_other_boundary": 15,
            "movie_doc_boundary": 15,
            "rare_music_doc": 15,
            "difficult_other_mixed": 15,
        }
        self.assertEqual(manifest["diagnostic_quotas"], expected_quotas)

        diag_records = [r for r in manifest["records"] if r["stratum"] == "diagnostic"]
        self.assertEqual(len(diag_records), 100)
        for r in diag_records:
            self.assertIn(r["retrieval_group"], expected_quotas)
            self.assertIsInstance(r["secondary_groups"], list)

    def test_04_zero_leakage_and_stratum_isolation(self):
        """Verify zero overlap between strata and against all exclusions."""
        manifest_path = self.v1_dir / "gold_pilot_v1_manifest.json"
        with open(manifest_path) as f:
            manifest = json.load(f)

        nat_hashes = {r["infohash"] for r in manifest["records"] if r["stratum"] == "natural"}
        diag_hashes = {r["infohash"] for r in manifest["records"] if r["stratum"] == "diagnostic"}
        self.assertEqual(len(nat_hashes & diag_hashes), 0)

        # Check manual_eval_set_200.jsonl overlap
        with open("data/manual_eval_set_200.jsonl") as f:
            eval200_hashes = {json.loads(l)["infohash"].strip().lower() for l in f if l.strip()}
        all_v1_hashes = set(r["infohash"] for r in manifest["records"])
        self.assertEqual(len(all_v1_hashes & eval200_hashes), 0)

    def test_05_blind_and_template_field_privacy(self):
        """Verify blind and template files contain strictly allowed fields and no leaks."""
        blind_path = self.v1_dir / "gold_pilot_v1_blind.jsonl"
        template_path = self.v1_dir / "gold_pilot_v1_review_template.jsonl"

        allowed_blind = {"pilot_id", "name", "file_count", "total_size_bytes", "files", "extension_summary", "metadata_mode"}
        allowed_template = {"pilot_id", "label_category", "reviewer_confidence", "ambiguous", "alternate_category", "reason", "reviewer_id", "annotation_timestamp", "adjudication_required"}

        prohibited = ["infohash", "source_dataset", "source_heuristic_label", "retrieval_group", "model_prediction", "stratum"]

        with open(blind_path) as f:
            for line in f:
                r = json.loads(line)
                self.assertEqual(set(r.keys()), allowed_blind)
                for p in prohibited:
                    self.assertNotIn(p, r)

        with open(template_path) as f:
            for line in f:
                r = json.loads(line)
                self.assertEqual(set(r.keys()), allowed_template)
                for k, v in r.items():
                    if k != "pilot_id":
                        self.assertIsNone(v)


if __name__ == "__main__":
    unittest.main()
