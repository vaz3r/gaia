#!/usr/bin/env python3
"""
Baseline v1 Data Preparation Script.
Constructs clean, frozen training and validation splits for the MiniLM baseline.
Enforces reference benchmark exclusion, duplicate deduplication, conflicting label removal,
and release-family-grouped stratification.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

# Add normalizer path
current_dir = Path(__file__).resolve().parent
sys.path.insert(0, str(current_dir / "audit"))
from release_normalizer import normalize_full_name, normalize_release_family

FROZEN_CLASSES = {
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
}

EXPECTED_REFERENCE_SHA256 = "16c45e847a4626a9ef468c1728bc1786949470a4cb2adb1ef12520ba4a6fb4f2"


def calc_sha256(path: Path | str) -> str:
    """Compute SHA-256 checksum of a file."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def compute_file_richness(row: dict) -> int:
    """Compute file richness score for deterministic representative selection."""
    top_dirs = row.get("top_dirs")
    if isinstance(top_dirs, list) and len(top_dirs) > 0:
        return len(top_dirs)
    files = row.get("files")
    if isinstance(files, list):
        return len(files)
    if isinstance(files, str) and files.strip():
        return len([f for f in files.split("|") if f.strip()])
    return 0


def load_reference_identities(
    reference_path: Path | str,
    manifest_path: Path | str,
    expected_ref_sha: str = EXPECTED_REFERENCE_SHA256,
) -> tuple[set[str], set[str], set[str]]:
    """
    Verify reference benchmark and return exclusion sets:
    (ref_hashes, ref_names, ref_families).
    """
    ref_path = Path(reference_path)
    man_path = Path(manifest_path)

    if not ref_path.exists():
        raise FileNotFoundError(f"Reference file not found: {ref_path}")
    if not man_path.exists():
        raise FileNotFoundError(f"Manifest file not found: {man_path}")

    actual_ref_sha = calc_sha256(ref_path)
    if actual_ref_sha != expected_ref_sha:
        raise ValueError(
            f"Reference checksum mismatch: expected {expected_ref_sha}, got {actual_ref_sha}"
        )

    with open(ref_path, "r", encoding="utf-8") as f:
        ref_records = [json.loads(line) for line in f if line.strip()]

    if len(ref_records) != 300:
        raise ValueError(f"Expected 300 reference records, got {len(ref_records)}")

    with open(man_path, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    manifest_pids = {r["pilot_id"]: r for r in manifest.get("records", [])}
    if len(manifest_pids) != 300:
        raise ValueError(f"Expected 300 manifest records, got {len(manifest_pids)}")

    ref_hashes = set()
    ref_names = set()
    ref_families = set()

    for r in ref_records:
        pid = r["pilot_id"]
        if pid not in manifest_pids:
            raise KeyError(f"Pilot ID {pid} from reference set not in manifest")
        infohash = manifest_pids[pid]["infohash"].lower().strip()
        name = r["name"]
        ref_hashes.add(infohash)
        ref_names.add(normalize_full_name(name))
        ref_families.add(normalize_release_family(name))

    return ref_hashes, ref_names, ref_families


def clean_training_data(
    source_path: Path | str,
    ref_hashes: set[str],
    ref_names: set[str],
    ref_families: set[str],
) -> tuple[list[dict], dict]:
    """
    Clean training data by filtering malformed rows, invalid labels, reference leaks,
    deduplicating exact hashes & normalized names, and excluding conflicting groups.
    """
    source_p = Path(source_path)
    if not source_p.exists():
        raise FileNotFoundError(f"Source training file not found: {source_p}")

    source_row_count = 0
    malformed_row_count = 0
    invalid_label_count = 0

    ref_excl_hash = 0
    ref_excl_name = 0
    ref_excl_family = 0

    valid_rows = []

    with open(source_p, "r", encoding="utf-8") as f:
        for idx, line in enumerate(f):
            if not line.strip():
                continue
            source_row_count += 1
            try:
                row = json.loads(line)
            except Exception:
                malformed_row_count += 1
                continue

            h = row.get("infohash")
            name = row.get("name")
            cat = row.get("label_category")

            if not isinstance(h, str) or not re.match(r"^[0-9a-fA-F]{40}$", h.strip()):
                malformed_row_count += 1
                continue
            if not name or not str(name).strip():
                malformed_row_count += 1
                continue

            h = h.lower().strip()
            name = str(name).strip()

            if cat not in FROZEN_CLASSES:
                invalid_label_count += 1
                continue

            norm_name = normalize_full_name(name)
            norm_fam = normalize_release_family(name)

            # Deterministic first-match priority reference exclusion
            if h in ref_hashes:
                ref_excl_hash += 1
                continue
            if norm_name in ref_names:
                ref_excl_name += 1
                continue
            if norm_fam in ref_families:
                ref_excl_family += 1
                continue

            row["source_row_index"] = idx
            row["infohash"] = h
            row["name"] = name
            row["norm_name"] = norm_name
            row["norm_family"] = norm_fam
            row["is_pseudo"] = bool(row.get("is_pseudo", False))
            row["sample_weight"] = float(
                row.get("sample_weight", 1.0) if row.get("sample_weight") is not None else 1.0
            )
            row["richness"] = compute_file_richness(row)

            valid_rows.append(row)

    # 4. Exact-infohash deduplication & conflict check
    hash_groups = defaultdict(list)
    for r in valid_rows:
        hash_groups[r["infohash"]].append(r)

    exact_dup_removals = 0
    conflicting_hash_groups = 0
    after_hash_rows = []

    for h, group in hash_groups.items():
        labels = set(x["label_category"] for x in group)
        if len(labels) > 1:
            conflicting_hash_groups += 1
            continue
        best = sorted(
            group,
            key=lambda x: (-x["richness"], -x["sample_weight"], x["source_row_index"]),
        )[0]
        exact_dup_removals += len(group) - 1
        after_hash_rows.append(best)

    # 5. Normalized-full-name deduplication & conflict check
    name_groups = defaultdict(list)
    for r in after_hash_rows:
        name_groups[r["norm_name"]].append(r)

    name_dup_removals = 0
    conflicting_name_groups = 0
    after_name_rows = []

    for nn, group in name_groups.items():
        labels = set(x["label_category"] for x in group)
        if len(labels) > 1:
            conflicting_name_groups += 1
            continue
        best = sorted(
            group,
            key=lambda x: (-x["richness"], -x["sample_weight"], x["source_row_index"]),
        )[0]
        name_dup_removals += len(group) - 1
        after_name_rows.append(best)

    # 6. Release-family grouping & conflict check
    fam_groups = defaultdict(list)
    for r in after_name_rows:
        fam_groups[r["norm_family"]].append(r)

    conflicting_fam_groups = 0
    clean_rows = []
    clean_fam_map = {}

    for fam, group in fam_groups.items():
        labels = set(x["label_category"] for x in group)
        if len(labels) > 1:
            conflicting_fam_groups += 1
            continue
        for r in group:
            r["release_family_group"] = fam
            clean_rows.append(r)
        clean_fam_map[fam] = group

    stats = {
        "source_row_count": source_row_count,
        "malformed_row_count": malformed_row_count,
        "invalid_label_count": invalid_label_count,
        "reference_exclusions_by_hash": ref_excl_hash,
        "reference_exclusions_by_name": ref_excl_name,
        "reference_exclusions_by_family": ref_excl_family,
        "exact_duplicate_removals": exact_dup_removals,
        "normalized_name_duplicate_removals": name_dup_removals,
        "conflicting_identity_groups_excluded": (
            conflicting_hash_groups + conflicting_name_groups + conflicting_fam_groups
        ),
        "clean_row_count": len(clean_rows),
        "clean_family_groups_count": len(clean_fam_map),
    }

    return clean_rows, stats


def split_release_families(
    clean_rows: list[dict],
    val_target_ratio: float = 0.15,
    random_seed: int = 42,
) -> tuple[list[dict], list[dict]]:
    """
    Perform deterministic grouped-stratified train/validation split by release family.
    Guarantees no release family, normalized name, or hash appears in both splits.
    Ensures all pseudo records remain in training.
    """
    rng = random.Random(random_seed)

    # Group by release family
    fam_groups = defaultdict(list)
    for r in clean_rows:
        fam_groups[r["release_family_group"]].append(r)

    # Group families by class
    class_groups = defaultdict(list)
    for fam, group in fam_groups.items():
        cat = group[0]["label_category"]
        has_pseudo = any(r.get("is_pseudo") for r in group)
        class_groups[cat].append((fam, group, has_pseudo))

    train_rows = []
    val_rows = []

    for cat in sorted(class_groups.keys()):
        groups = class_groups[cat]
        forced_train = [g for g in groups if g[2]]
        eligible = [g for g in groups if not g[2]]

        for fam, group, _ in forced_train:
            train_rows.extend(group)

        # Deterministic sort then shuffle
        eligible.sort(key=lambda g: g[0])
        rng.shuffle(eligible)

        total_class_rows = sum(len(g[1]) for g in groups)
        target_val_rows = max(1, round(total_class_rows * val_target_ratio))

        curr_val_rows = 0
        for fam, group, _ in eligible:
            if curr_val_rows < target_val_rows:
                val_rows.extend(group)
                curr_val_rows += len(group)
            else:
                train_rows.extend(group)

    # Stable deterministic ordering
    train_rows.sort(key=lambda r: r["source_row_index"])
    val_rows.sort(key=lambda r: r["source_row_index"])

    return train_rows, val_rows


def prepare_baseline_data(
    source_path: str = "apps/classifier/data/training_combined_v10_true.jsonl",
    reference_path: str = "apps/classifier/data/gold_pilot_v1/reference_eval_v1.jsonl",
    manifest_path: str = "apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json",
    out_dir: str = "apps/classifier/data/baseline_v1",
    random_seed: int = 42,
) -> dict:
    """Execute full data preparation pipeline and write outputs."""
    out_p = Path(out_dir)
    out_p.mkdir(parents=True, exist_ok=True)

    normalizer_path = Path("apps/classifier/tools/audit/release_normalizer.py")
    norm_sha = calc_sha256(normalizer_path) if normalizer_path.exists() else ""

    ref_hashes, ref_names, ref_families = load_reference_identities(
        reference_path, manifest_path
    )

    clean_rows, clean_stats = clean_training_data(
        source_path, ref_hashes, ref_names, ref_families
    )

    train_rows, val_rows = split_release_families(
        clean_rows, val_target_ratio=0.15, random_seed=random_seed
    )

    train_file = out_p / "train.jsonl"
    val_file = out_p / "validation.jsonl"
    split_manifest_file = out_p / "split_manifest.json"

    def format_row(r: dict) -> dict:
        out_r = {
            "infohash": r["infohash"],
            "name": r["name"],
            "file_count": r.get("file_count", 1),
            "total_size_bytes": r.get("total_size_bytes", r.get("total_size", 0)),
            "files": r.get("files", ""),
            "top_dirs": r.get("top_dirs", []),
            "label_category": r["label_category"],
            "is_pseudo": r.get("is_pseudo", False),
            "sample_weight": r.get("sample_weight", 1.0),
            "source_row_index": r["source_row_index"],
            "release_family_group": r["release_family_group"],
        }
        if "heuristic_hint" in r:
            out_r["heuristic_hint"] = r["heuristic_hint"]
        return out_r

    with open(train_file, "w", encoding="utf-8") as f:
        for r in train_rows:
            f.write(json.dumps(format_row(r), ensure_ascii=False) + "\n")

    with open(val_file, "w", encoding="utf-8") as f:
        for r in val_rows:
            f.write(json.dumps(format_row(r), ensure_ascii=False) + "\n")

    train_sha = calc_sha256(train_file)
    val_sha = calc_sha256(val_file)

    train_hashes = set(r["infohash"] for r in train_rows)
    val_hashes = set(r["infohash"] for r in val_rows)
    train_names = set(r["norm_name"] for r in train_rows)
    val_names = set(r["norm_name"] for r in val_rows)
    train_fams = set(r["norm_family"] for r in train_rows)
    val_fams = set(r["norm_family"] for r in val_rows)

    all_dataset_hashes = train_hashes | val_hashes
    all_dataset_names = train_names | val_names
    all_dataset_fams = train_fams | val_fams

    manifest_data = {
        "version": "baseline-v1",
        "random_seed": random_seed,
        "source_dataset_path": str(source_path),
        "source_dataset_sha256": calc_sha256(source_path),
        "reference_dataset_path": str(reference_path),
        "reference_dataset_sha256": calc_sha256(reference_path),
        "normalization_implementation_checksum": norm_sha,
        "source_row_count": clean_stats["source_row_count"],
        "malformed_row_count": clean_stats["malformed_row_count"],
        "invalid_label_count": clean_stats["invalid_label_count"],
        "reference_exclusions_by_hash": clean_stats["reference_exclusions_by_hash"],
        "reference_exclusions_by_name": clean_stats["reference_exclusions_by_name"],
        "reference_exclusions_by_family": clean_stats["reference_exclusions_by_family"],
        "exact_duplicate_removals": clean_stats["exact_duplicate_removals"],
        "normalized_name_duplicate_removals": clean_stats["normalized_name_duplicate_removals"],
        "conflicting_identity_groups_excluded": clean_stats["conflicting_identity_groups_excluded"],
        "clean_row_count": clean_stats["clean_row_count"],
        "training_row_count": len(train_rows),
        "validation_row_count": len(val_rows),
        "train_class_counts": dict(Counter(r["label_category"] for r in train_rows)),
        "validation_class_counts": dict(Counter(r["label_category"] for r in val_rows)),
        "train_pseudo_vs_non_pseudo_counts": {
            "pseudo": sum(1 for r in train_rows if r["is_pseudo"]),
            "non_pseudo": sum(1 for r in train_rows if not r["is_pseudo"]),
        },
        "validation_pseudo_vs_non_pseudo_counts": {
            "pseudo": sum(1 for r in val_rows if r["is_pseudo"]),
            "non_pseudo": sum(1 for r in val_rows if not r["is_pseudo"]),
        },
        "train_sample_weight_distribution": dict(
            Counter(str(r["sample_weight"]) for r in train_rows)
        ),
        "validation_sample_weight_distribution": dict(
            Counter(str(r["sample_weight"]) for r in val_rows)
        ),
        "cross_split_overlap_counts": {
            "hash_overlap": len(train_hashes & val_hashes),
            "name_overlap": len(train_names & val_names),
            "family_overlap": len(train_fams & val_fams),
        },
        "reference_overlap_counts": {
            "hash_overlap": len(all_dataset_hashes & ref_hashes),
            "name_overlap": len(all_dataset_names & ref_names),
            "family_overlap": len(all_dataset_fams & ref_families),
        },
        "train_file_sha256": train_sha,
        "validation_file_sha256": val_sha,
        "creation_timestamp": datetime.now(timezone.utc).isoformat(),
    }

    with open(split_manifest_file, "w", encoding="utf-8") as f:
        json.dump(manifest_data, f, indent=2, ensure_ascii=False)

    return manifest_data


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Prepare clean baseline v1 splits.")
    parser.add_argument(
        "--source",
        default="apps/classifier/data/training_combined_v10_true.jsonl",
        help="Path to source training JSONL",
    )
    parser.add_argument(
        "--reference",
        default="apps/classifier/data/gold_pilot_v1/reference_eval_v1.jsonl",
        help="Path to reference evaluation JSONL",
    )
    parser.add_argument(
        "--manifest",
        default="apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json",
        help="Path to gold pilot manifest JSON",
    )
    parser.add_argument(
        "--out_dir",
        default="apps/classifier/data/baseline_v1",
        help="Output directory for splits and manifest",
    )
    parser.add_argument("--seed", type=int, default=42, help="Random seed")
    args = parser.parse_args()

    res = prepare_baseline_data(
        source_path=args.source,
        reference_path=args.reference,
        manifest_path=args.manifest,
        out_dir=args.out_dir,
        random_seed=args.seed,
    )
    print("Baseline v1 data preparation complete.")
