#!/usr/bin/env python3
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

apps_dir = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(apps_dir / "tools"))
sys.path.insert(0, str(apps_dir / "tools" / "audit"))

from prepare_baseline_v1_data import (
    calc_sha256,
    load_reference_identities,
    clean_training_data,
    split_release_families,
    prepare_baseline_data,
    FROZEN_CLASSES,
    EXPECTED_REFERENCE_SHA256,
)


class TestPrepareBaselineV1Data(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.td = Path(self.temp_dir.name)

        self.ref_file = self.td / "reference_eval_v1.jsonl"
        self.manifest_file = self.td / "gold_pilot_v1_manifest.json"

        self.dummy_refs = []
        self.dummy_manifest_records = []
        for i in range(300):
            pid = f"GP1-{i:016X}"
            h = f"{i:040x}"
            name = f"Reference.Release.{i}.2024.1080p.mkv"
            self.dummy_refs.append({
                "pilot_id": pid,
                "name": name,
                "file_count": 1,
                "total_size_bytes": 1000,
                "files": [],
                "extension_summary": {},
                "metadata_mode": "sparse_single_file",
                "label_category": "Movies",
                "label_resolution": "exact_dual_pass_agreement",
                "reference_confidence": "consensus"
            })
            self.dummy_manifest_records.append({
                "pilot_id": pid,
                "infohash": h,
                "name": name
            })

        with open(self.ref_file, "w", encoding="utf-8") as f:
            for r in self.dummy_refs:
                f.write(json.dumps(r) + "\n")

        with open(self.manifest_file, "w", encoding="utf-8") as f:
            json.dump({"records": self.dummy_manifest_records}, f)

        self.custom_ref_sha = calc_sha256(self.ref_file)

    def tearDown(self):
        self.temp_dir.cleanup()

    def test_01_deterministic_output(self):
        src_file = self.td / "test_src.jsonl"
        rows = [
            {"infohash": f"a{i:039x}", "name": f"Item {i} 1080p", "label_category": "Anime", "sample_weight": 1.0, "is_pseudo": False, "top_dirs": []}
            for i in range(50)
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean1, _ = clean_training_data(src_file, ref_h, ref_n, ref_f)
        clean2, _ = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(clean1, clean2)

        t1, v1 = split_release_families(clean1, val_target_ratio=0.2, random_seed=42)
        t2, v2 = split_release_families(clean2, val_target_ratio=0.2, random_seed=42)
        self.assertEqual(t1, t2)
        self.assertEqual(v1, v2)

    def test_02_reference_checksum_enforcement(self):
        with self.assertRaises(ValueError):
            load_reference_identities(self.ref_file, self.manifest_file, expected_ref_sha="bad_checksum")

    def test_03_exact_hash_benchmark_exclusion(self):
        src_file = self.td / "src.jsonl"
        ref_h0 = f"{0:040x}"
        rows = [
            {"infohash": ref_h0, "name": "Unrelated Name 1080p", "label_category": "Anime"},
            {"infohash": f"b{1:039x}", "name": "Allowed Name 1080p", "label_category": "Anime"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["reference_exclusions_by_hash"], 1)
        self.assertEqual(len(clean), 1)
        self.assertEqual(clean[0]["name"], "Allowed Name 1080p")

    def test_04_normalized_name_benchmark_exclusion(self):
        src_file = self.td / "src.jsonl"
        rows = [
            {"infohash": f"c{0:039x}", "name": "Reference.Release.0.2024.1080p.mkv", "label_category": "Movies"},
            {"infohash": f"c{1:039x}", "name": "Clean Movie 2024.mkv", "label_category": "Movies"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["reference_exclusions_by_name"], 1)
        self.assertEqual(len(clean), 1)
        self.assertEqual(clean[0]["name"], "Clean Movie 2024.mkv")

    def test_05_release_family_benchmark_exclusion(self):
        src_file = self.td / "src.jsonl"
        rows = [
            {"infohash": f"d{0:039x}", "name": "Reference.Release.0.2024.720p.BluRay.x264", "label_category": "Movies"},
            {"infohash": f"d{1:039x}", "name": "Clean Movie 2024.mkv", "label_category": "Movies"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["reference_exclusions_by_family"], 1)
        self.assertEqual(len(clean), 1)

    def test_06_exact_duplicate_removal(self):
        src_file = self.td / "src.jsonl"
        h = f"e{0:039x}"
        rows = [
            {"infohash": h, "name": "Item Name 1", "label_category": "Games", "top_dirs": []},
            {"infohash": h, "name": "Item Name 1", "label_category": "Games", "top_dirs": []},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["exact_duplicate_removals"], 1)
        self.assertEqual(len(clean), 1)

    def test_07_richer_metadata_representative_preference(self):
        src_file = self.td / "src.jsonl"
        h = f"f{0:039x}"
        rows = [
            {"infohash": h, "name": "Item Name", "label_category": "Applications", "top_dirs": [], "files": ""},
            {"infohash": h, "name": "Item Name", "label_category": "Applications", "top_dirs": ["setup.exe", "crack.dll"], "files": "setup.exe | crack.dll"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, _ = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(len(clean), 1)
        self.assertEqual(clean[0]["top_dirs"], ["setup.exe", "crack.dll"])

    def test_08_sample_weight_representative_preference(self):
        src_file = self.td / "src.jsonl"
        h = f"1{0:039x}"
        rows = [
            {"infohash": h, "name": "Item Name", "label_category": "Music", "sample_weight": 0.5, "top_dirs": []},
            {"infohash": h, "name": "Item Name", "label_category": "Music", "sample_weight": 2.0, "top_dirs": []},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, _ = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(len(clean), 1)
        self.assertEqual(clean[0]["sample_weight"], 2.0)

    def test_09_conflicting_label_group_removal(self):
        src_file = self.td / "src.jsonl"
        h = f"2{0:039x}"
        rows = [
            {"infohash": h, "name": "Ambiguous Release S01E01", "label_category": "Television"},
            {"infohash": h, "name": "Ambiguous Release S01E01", "label_category": "Anime"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["conflicting_identity_groups_excluded"], 1)
        self.assertEqual(len(clean), 0)

    def test_10_pseudo_records_cannot_enter_validation(self):
        clean_rows = [
            {"infohash": f"3{i:039x}", "name": f"Pseudo Show S01E{i:02d}", "norm_name": f"pseudo show s01e{i:02d}", "norm_family": f"pseudo show s01e{i:02d}", "release_family_group": f"pseudo show s01e{i:02d}", "label_category": "Television", "is_pseudo": True, "source_row_index": i}
            for i in range(10)
        ] + [
            {"infohash": f"4{i:039x}", "name": f"Real Show S01E{i:02d}", "norm_name": f"real show s01e{i:02d}", "norm_family": f"real show s01e{i:02d}", "release_family_group": f"real show s01e{i:02d}", "label_category": "Television", "is_pseudo": False, "source_row_index": 10 + i}
            for i in range(10)
        ]
        train, val = split_release_families(clean_rows, val_target_ratio=0.3, random_seed=42)
        self.assertTrue(all(r["is_pseudo"] is False for r in val))
        self.assertEqual(sum(1 for r in train if r["is_pseudo"]), 10)

    def test_11_no_release_family_crosses_split(self):
        clean_rows = [
            {"infohash": f"5{i:039x}", "name": f"Series A S01E{i:02d}", "norm_name": f"series a s01e{i:02d}", "norm_family": "series a", "release_family_group": "series a", "label_category": "Television", "is_pseudo": False, "source_row_index": i}
            for i in range(4)
        ] + [
            {"infohash": f"6{i:039x}", "name": f"Series B S01E{i:02d}", "norm_name": f"series b s01e{i:02d}", "norm_family": "series b", "release_family_group": "series b", "label_category": "Television", "is_pseudo": False, "source_row_index": 4 + i}
            for i in range(4)
        ]
        train, val = split_release_families(clean_rows, val_target_ratio=0.5, random_seed=42)
        train_fams = set(r["release_family_group"] for r in train)
        val_fams = set(r["release_family_group"] for r in val)
        self.assertEqual(len(train_fams & val_fams), 0)

    def test_12_no_normalized_name_crosses_split(self):
        clean_rows = [
            {"infohash": f"7{i:039x}", "name": f"Unique Movie {i}", "norm_name": f"unique movie {i}", "norm_family": f"unique movie {i}", "release_family_group": f"unique movie {i}", "label_category": "Movies", "is_pseudo": False, "source_row_index": i}
            for i in range(20)
        ]
        train, val = split_release_families(clean_rows, val_target_ratio=0.2, random_seed=42)
        train_names = set(r["norm_name"] for r in train)
        val_names = set(r["norm_name"] for r in val)
        self.assertEqual(len(train_names & val_names), 0)

    def test_13_no_exact_hash_crosses_split(self):
        clean_rows = [
            {"infohash": f"8{i:039x}", "name": f"Track {i}", "norm_name": f"track {i}", "norm_family": f"track {i}", "release_family_group": f"track {i}", "label_category": "Music", "is_pseudo": False, "source_row_index": i}
            for i in range(20)
        ]
        train, val = split_release_families(clean_rows, val_target_ratio=0.2, random_seed=42)
        train_h = set(r["infohash"] for r in train)
        val_h = set(r["infohash"] for r in val)
        self.assertEqual(len(train_h & val_h), 0)

    def test_14_all_eight_valid_labels_accepted(self):
        src_file = self.td / "src.jsonl"
        rows = [
            {"infohash": f"9{i:039x}", "name": f"Valid {cat}", "label_category": cat}
            for i, cat in enumerate(sorted(list(FROZEN_CLASSES)))
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(len(clean), 8)
        self.assertEqual(stats["invalid_label_count"], 0)

    def test_15_malformed_and_invalid_label_rows_rejected(self):
        src_file = self.td / "src.jsonl"
        rows = [
            {"infohash": "invalid_hash_len", "name": "Bad Hash", "label_category": "Anime"},
            {"infohash": f"a{1:039x}", "name": "", "label_category": "Anime"},
            {"infohash": f"a{2:039x}", "name": "Bad Category", "label_category": "UnknownCategory"},
            {"infohash": f"a{3:039x}", "name": "Valid Category", "label_category": "Anime"},
        ]
        with open(src_file, "w", encoding="utf-8") as f:
            f.write("not a json line\n")
            for r in rows:
                f.write(json.dumps(r) + "\n")

        ref_h, ref_n, ref_f = load_reference_identities(self.ref_file, self.manifest_file, self.custom_ref_sha)
        clean, stats = clean_training_data(src_file, ref_h, ref_n, ref_f)
        self.assertEqual(stats["malformed_row_count"], 3)
        self.assertEqual(stats["invalid_label_count"], 1)
        self.assertEqual(len(clean), 1)

    def test_16_source_datasets_remain_unchanged(self):
        real_src = Path("apps/classifier/data/training_combined_v10_true.jsonl")
        real_ref = Path("apps/classifier/data/gold_pilot_v1/reference_eval_v1.jsonl")
        if real_src.exists() and real_ref.exists():
            orig_src_sha = calc_sha256(real_src)
            orig_ref_sha = calc_sha256(real_ref)

            prepare_baseline_data(
                source_path=str(real_src),
                reference_path=str(real_ref),
                manifest_path="apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json",
                out_dir=str(self.td / "baseline_out"),
                random_seed=42,
            )

            self.assertEqual(calc_sha256(real_src), orig_src_sha)
            self.assertEqual(calc_sha256(real_ref), orig_ref_sha)


if __name__ == "__main__":
    unittest.main()
