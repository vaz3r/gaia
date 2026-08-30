#!/usr/bin/env python3
"""
Gold Pilot Dataset Builder.
Constructs a strictly deduplicated, 300-record human-annotation pilot
with two strata (Natural & Diagnostic) free from training leakage.
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

# Add audit tools directory for release normalization
sys.path.insert(0, str(Path(__file__).resolve().parent / "audit"))
from release_normalizer import normalize_full_name, normalize_release_family

NAMESPACE = "gaia_gold_pilot_v1"
SEED = 42

TRAINING_DATASETS = [
    "apps/classifier/data/training_combined_v10_true.jsonl",
    "apps/classifier/data/training_combined_v9_true.jsonl",
    "apps/classifier/data/manual_seed_1800_labeled.jsonl",
    "apps/classifier/data/labeled.jsonl",
    "apps/classifier/data/remote_extra.jsonl",
    "apps/classifier/data/remote_legit.jsonl",
    "apps/classifier/data/remote_porn.jsonl",
    "apps/classifier/data/remote_porn_western.jsonl",
    "apps/classifier/data/labeled_150.jsonl",
    "apps/classifier/data/labeling_sample_final.jsonl",
]

DIAGNOSTIC_SOURCES = [
    "apps/classifier/data/manual_eval_set_balanced_2000.jsonl",
    "apps/classifier/data/al_labeled_1129_true.jsonl",
    "apps/classifier/data/edge_cases_large.jsonl",
    "apps/classifier/data/edge_cases.jsonl",
]


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def generate_pilot_id(infohash: str, index: int) -> str:
    """Generate a collision-safe deterministic opaque identifier: GP1-XXXXXXXXXXXXXXXX."""
    payload = f"{NAMESPACE}:{infohash}:{index}".encode("utf-8")
    digest = hashlib.sha256(payload).hexdigest()[:16].upper()
    return f"GP1-{digest}"


def extract_extension_summary(files: list[str]) -> dict[str, int]:
    """Compute file extension counts directly from file paths."""
    counts: dict[str, int] = defaultdict(int)
    for f in files:
        dot = f.rfind(".")
        if dot >= 0:
            ext = f[dot:].lower()
            if len(ext) <= 10 and re.match(r"^\.[a-z0-9]+$", ext):
                counts[ext] += 1
    return dict(sorted(counts.items(), key=lambda x: -x[1]))


def assess_metadata_completeness(name: str, file_count: int, total_size_bytes: int, files: list[str]) -> dict:
    """Evaluate completeness of inference-visible metadata."""
    has_name = bool(name and name.strip() and name.strip() != "[unknown]")
    has_files = bool(files and len(files) > 0)
    has_valid_count = bool(file_count and int(file_count) > 0)
    has_valid_size = bool(total_size_bytes and int(total_size_bytes) > 0)
    
    is_complete = has_name and has_files and has_valid_count and has_valid_size
    
    return {
        "is_complete": is_complete,
        "has_name": has_name,
        "has_files": has_files,
        "has_valid_count": has_valid_count,
        "has_valid_size": has_valid_size,
    }


def load_training_exclusions() -> tuple[set[str], set[str], set[str], dict[str, str]]:
    """Load all training records and return sets of infohashes, normalized names, and release families."""
    train_hashes = set()
    train_names = set()
    train_families = set()
    train_checksums = {}

    for tf in TRAINING_DATASETS:
        if os.path.exists(tf):
            train_checksums[tf] = sha256_file(tf)
            with open(tf, "r", encoding="utf-8", errors="ignore") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        r = json.loads(line)
                        ih = r.get("infohash", r.get("id", "")).strip().lower()
                        nm = r.get("name", "").strip()
                        if ih:
                            train_hashes.add(ih)
                        if nm:
                            train_names.add(normalize_full_name(nm))
                            train_families.add(normalize_release_family(nm))
                    except Exception:
                        pass

    return train_hashes, train_names, train_families, train_checksums


def select_natural_stratum(
    eval_file: str,
    train_hashes: set[str],
    train_names: set[str],
    train_families: set[str],
    target_count: int = 200,
    seed: int = SEED,
) -> tuple[list[dict], dict[str, int]]:
    """Select leak-free random sample from natural evaluation set."""
    with open(eval_file, "r", encoding="utf-8") as f:
        records = [json.loads(l) for l in f if l.strip()]

    rng = random.Random(seed)
    # Shuffle entire list deterministically
    indices = list(range(len(records)))
    rng.shuffle(indices)

    selected = []
    seen_hashes = set()
    seen_names = set()
    seen_families = set()
    exclusion_counts = Counter()

    for idx in indices:
        r = records[idx]
        ih = r.get("infohash", r.get("id", "")).strip().lower()
        nm = r.get("name", "").strip()
        nn = normalize_full_name(nm)
        rf = normalize_release_family(nm)

        # Exclusion checks against training data
        if not ih or ih in train_hashes:
            exclusion_counts["training_exact_hash"] += 1
            continue
        if nn in train_names:
            exclusion_counts["training_normalized_name"] += 1
            continue
        if rf in train_families:
            exclusion_counts["training_release_family"] += 1
            continue

        # Deduplication within stratum
        if ih in seen_hashes:
            exclusion_counts["internal_duplicate_hash"] += 1
            continue
        if nn in seen_names:
            exclusion_counts["internal_duplicate_name"] += 1
            continue
        if rf in seen_families:
            exclusion_counts["internal_duplicate_release_family"] += 1
            continue

        seen_hashes.add(ih)
        seen_names.add(nn)
        seen_families.add(rf)

        r["__source_dataset__"] = eval_file
        r["__source_row_idx__"] = idx
        r["__stratum__"] = "natural"
        r["__retrieval_group__"] = "natural_random"
        selected.append(r)

        if len(selected) >= target_count:
            break

    return selected, dict(exclusion_counts)


def select_diagnostic_stratum(
    train_hashes: set[str],
    train_names: set[str],
    train_families: set[str],
    natural_hashes: set[str],
    natural_names: set[str],
    natural_families: set[str],
    target_count: int = 100,
    seed: int = SEED,
) -> tuple[list[dict], dict[str, int], dict[str, int]]:
    """Select diagnostic candidates targeting boundary confusions and rare classes."""
    pool = []
    for sf in DIAGNOSTIC_SOURCES:
        if os.path.exists(sf):
            with open(sf, "r", encoding="utf-8", errors="ignore") as f:
                for i, line in enumerate(f):
                    if not line.strip():
                        continue
                    try:
                        r = json.loads(line)
                        r["__source_dataset__"] = sf
                        r["__source_row_idx__"] = i
                        pool.append(r)
                    except Exception:
                        pass

    rng = random.Random(seed)
    rng.shuffle(pool)

    # Retrieval group targets
    group_targets = {
        "anime_tv_boundary": 20,
        "app_game_boundary": 20,
        "game_other_boundary": 15,
        "movie_doc_boundary": 15,
        "rare_music_doc": 15,
        "difficult_other_mixed": 15,
    }

    group_collected = defaultdict(list)
    seen_hashes = set(natural_hashes)
    seen_names = set(natural_names)
    seen_families = set(natural_families)
    exclusion_counts = Counter()

    for r in pool:
        ih = r.get("infohash", r.get("id", "")).strip().lower()
        nm = r.get("name", "").strip()
        nn = normalize_full_name(nm)
        rf = normalize_release_family(nm)
        cat = r.get("label_category") or r.get("category") or "Other"

        # Leakage checks against training
        if not ih or ih in train_hashes:
            exclusion_counts["training_exact_hash"] += 1
            continue
        if nn in train_names:
            exclusion_counts["training_normalized_name"] += 1
            continue
        if rf in train_families:
            exclusion_counts["training_release_family"] += 1
            continue

        # Overlap checks against natural stratum and within diagnostic stratum
        if ih in seen_hashes:
            exclusion_counts["stratum_overlap_hash"] += 1
            continue
        if nn in seen_names:
            exclusion_counts["stratum_overlap_name"] += 1
            continue
        if rf in seen_families:
            exclusion_counts["stratum_overlap_release_family"] += 1
            continue

        name_l = nm.lower()
        assigned_group = None

        # Check candidate eligibility for each group
        if (cat in ("Anime", "Television") or re.search(r"s\d{1,2}e\d{1,3}|season|\[subs|fansub|anime", name_l)) and len(group_collected["anime_tv_boundary"]) < group_targets["anime_tv_boundary"]:
            assigned_group = "anime_tv_boundary"
        elif (cat in ("Applications", "Games") or re.search(r"setup\.exe|keygen|crack|portable|patch|repack|fitgirl|dodi|iso|dmg", name_l)) and len(group_collected["app_game_boundary"]) < group_targets["app_game_boundary"]:
            assigned_group = "app_game_boundary"
        elif (cat in ("Games", "Other") or re.search(r"rom|nsp|xci|pkg|vpk|cia|gog|steam", name_l)) and len(group_collected["game_other_boundary"]) < group_targets["game_other_boundary"]:
            assigned_group = "game_other_boundary"
        elif (cat in ("Movies", "Documentaries") or re.search(r"bbc|pbs|nova|natgeo|documentary|1080p|720p|bluray", name_l)) and len(group_collected["movie_doc_boundary"]) < group_targets["movie_doc_boundary"]:
            assigned_group = "movie_doc_boundary"
        elif (cat in ("Music", "Documentaries") or re.search(r"flac|mp3|discography|album|soundtrack|ost", name_l)) and len(group_collected["rare_music_doc"]) < group_targets["rare_music_doc"]:
            assigned_group = "rare_music_doc"
        elif (cat == "Other" or len(nm) <= 15 or re.search(r"archive|collection|mixed|junk|course|book|pdf|rar|7z|zip", name_l)) and len(group_collected["difficult_other_mixed"]) < group_targets["difficult_other_mixed"]:
            assigned_group = "difficult_other_mixed"

        if assigned_group:
            seen_hashes.add(ih)
            seen_names.add(nn)
            seen_families.add(rf)
            r["__stratum__"] = "diagnostic"
            r["__retrieval_group__"] = assigned_group
            group_collected[assigned_group].append(r)

        if sum(len(items) for items in group_collected.values()) >= target_count:
            break

    selected = []
    for g in group_targets.keys():
        selected.extend(group_collected[g])

    group_counts = {g: len(items) for g, items in group_collected.items()}
    return selected, group_counts, dict(exclusion_counts)


def build_gold_pilot(out_dir: str = "apps/classifier/data/gold_pilot") -> dict:
    """Build blind pilot, review template, and internal manifest."""
    out_path = Path(out_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    print("=================================================================")
    print("BUILDING HUMAN-ANNOTATION GOLD PILOT (300 SAMPLES)")
    print("=================================================================")

    # 1. Load Training Exclusions
    print("\n1. Loading training datasets to ensure zero leakage...")
    train_hashes, train_names, train_families, train_checksums = load_training_exclusions()
    print(f"   Loaded {len(train_hashes)} unique training infohashes.")
    print(f"   Loaded {len(train_names)} unique training normalized names.")
    print(f"   Loaded {len(train_families)} unique training release families.")

    # 2. Select Stratum A (Natural Pilot - 200 samples)
    print("\n2. Sampling Stratum A (Natural Distribution, 200 records)...")
    eval_1000_path = "apps/classifier/data/manual_eval_set_1000.jsonl"
    eval_1000_checksum = sha256_file(eval_1000_path)
    natural_samples, natural_exclusions = select_natural_stratum(
        eval_1000_path, train_hashes, train_names, train_families, target_count=200, seed=SEED
    )
    print(f"   Selected {len(natural_samples)} natural records (Seed={SEED}).")
    print(f"   Natural exclusions: {natural_exclusions}")

    natural_hashes = set(r["infohash"].strip().lower() for r in natural_samples)
    natural_names = set(normalize_full_name(r["name"]) for r in natural_samples)
    natural_families = set(normalize_release_family(r["name"]) for r in natural_samples)

    # 3. Select Stratum B (Diagnostic Pilot - 100 samples)
    print("\n3. Sampling Stratum B (Diagnostic Boundary & Rare, 100 records)...")
    diagnostic_samples, diag_group_counts, diag_exclusions = select_diagnostic_stratum(
        train_hashes,
        train_names,
        train_families,
        natural_hashes,
        natural_names,
        natural_families,
        target_count=100,
        seed=SEED,
    )
    print(f"   Selected {len(diagnostic_samples)} diagnostic records.")
    print(f"   Diagnostic groups: {diag_group_counts}")
    print(f"   Diagnostic exclusions: {diag_exclusions}")

    # Combine pilot records
    all_pilot_records = natural_samples + diagnostic_samples
    assert len(all_pilot_records) == 300, f"Expected 300 records, got {len(all_pilot_records)}"

    # 4. Generate Blind Records, Review Template, and Manifest Entries
    print("\n4. Generating blind files, review templates, and manifest...")
    blind_records = []
    template_records = []
    manifest_records = []
    pilot_ids = set()

    completeness_stats = Counter()

    for i, r in enumerate(all_pilot_records):
        ih = r.get("infohash", r.get("id", "")).strip().lower()
        name = r.get("name", "").strip()
        fc = int(r.get("file_count", 0))
        size = int(r.get("total_size", r.get("total_size_bytes", 0)))
        files = r.get("top_dirs", r.get("files", [])) or []

        if isinstance(files, list) and len(files) > 0 and isinstance(files[0], list):
            files = ["/".join(str(p) for p in f) for f in files]
        elif isinstance(files, list) and len(files) > 0 and isinstance(files[0], dict):
            files = [f.get("path", "") for f in files]

        pilot_id = generate_pilot_id(ih, i)
        assert pilot_id not in pilot_ids, f"Collision detected for pilot ID: {pilot_id}"
        pilot_ids.add(pilot_id)

        ext_summary = extract_extension_summary(files)
        comp = assess_metadata_completeness(name, fc, size, files)
        if comp["is_complete"]:
            completeness_stats["complete"] += 1
        else:
            completeness_stats["incomplete"] += 1
        if not comp["has_name"]:
            completeness_stats["empty_name"] += 1
        if not comp["has_files"]:
            completeness_stats["empty_files"] += 1
        if not comp["has_valid_count"]:
            completeness_stats["zero_file_count"] += 1
        if not comp["has_valid_size"]:
            completeness_stats["zero_size"] += 1

        # Blind record (Reviewer visible only)
        blind_records.append({
            "pilot_id": pilot_id,
            "name": name,
            "file_count": fc,
            "total_size_bytes": size,
            "files": files[:20],
            "extension_summary": ext_summary,
            "metadata_completeness": comp,
        })

        # Review Template record
        template_records.append({
            "pilot_id": pilot_id,
            "label_category": None,
            "reviewer_confidence": None,
            "ambiguous": None,
            "alternate_category": None,
            "reason": None,
            "reviewer_id": None,
            "annotation_timestamp": None,
            "adjudication_required": None,
        })

        # Manifest internal record
        manifest_records.append({
            "pilot_id": pilot_id,
            "infohash": ih,
            "stratum": r["__stratum__"],
            "retrieval_group": r["__retrieval_group__"],
            "source_dataset": r["__source_dataset__"],
            "source_row_idx": r["__source_row_idx__"],
            "source_heuristic_label": r.get("label_category") or r.get("category"),
            "normalized_name_hash": hashlib.sha256(normalize_full_name(name).encode()).hexdigest(),
            "release_family_hash": hashlib.sha256(normalize_release_family(name).encode()).hexdigest(),
        })

    # Write files
    blind_path = out_path / "gold_pilot_blind.jsonl"
    with open(blind_path, "w", encoding="utf-8") as f:
        for r in blind_records:
            f.write(json.dumps(r) + "\n")

    template_path = out_path / "gold_pilot_review_template.jsonl"
    with open(template_path, "w", encoding="utf-8") as f:
        for r in template_records:
            f.write(json.dumps(r) + "\n")

    manifest = {
        "pilot_version": "1.0",
        "generation_timestamp": datetime.now(timezone.utc).isoformat(),
        "random_seed": SEED,
        "sample_counts": {
            "requested_total": 300,
            "actual_total": len(all_pilot_records),
            "natural_stratum": len(natural_samples),
            "diagnostic_stratum": len(diagnostic_samples),
        },
        "diagnostic_group_counts": diag_group_counts,
        "metadata_completeness_summary": dict(completeness_stats),
        "exclusion_summary": {
            "natural_stratum": natural_exclusions,
            "diagnostic_stratum": diag_exclusions,
        },
        "source_checksums": {
            "eval_1000": eval_1000_checksum,
            "training_datasets": train_checksums,
        },
        "output_checksums": {
            "gold_pilot_blind_sha256": sha256_file(str(blind_path)),
            "gold_pilot_review_template_sha256": sha256_file(str(template_path)),
        },
        "records": manifest_records,
    }

    manifest_path = out_path / "gold_pilot_manifest.json"
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)

    print(f"\n✅ Pilot successfully generated at {out_path}:")
    print(f"   - Blind File: {blind_path} (SHA-256: {manifest['output_checksums']['gold_pilot_blind_sha256'][:16]}...)")
    print(f"   - Template File: {template_path} (SHA-256: {manifest['output_checksums']['gold_pilot_review_template_sha256'][:16]}...)")
    print(f"   - Manifest File: {manifest_path}")
    print(f"   - Metadata Completeness: {dict(completeness_stats)}")

    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Build 300-record gold annotation pilot.")
    parser.add_argument("--out_dir", default="apps/classifier/data/gold_pilot", help="Output directory")
    args = parser.parse_args()
    build_gold_pilot(args.out_dir)
