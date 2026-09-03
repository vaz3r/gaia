#!/usr/bin/env python3
"""
Gold Pilot v1 Dataset Builder.
Finalizes Gold Pilot v1 with:
- Stratum A: 200 clean, leak-free natural records (sparse single-file).
- Stratum B: 100 fresh, leak-free diagnostic records (rich multi-file from candidate_pool_rich.jsonl).
Total: exactly 300 records.
Preserves Gold Pilot v0 completely untouched.
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

NAMESPACE_V1 = "gaia_gold_pilot_v1"
SEED = 42

V0_CHECKSUMS = {
    "manifest": "49c7d867718a0e996413544d194b1ca48cc60035402085373b6e80dc27af9ff5",
    "blind": "4f2abbeb87f9f9b3c04f00d06d9324b275fe4b057f5467974e909fa471a07ad0",
    "template": "a64353f68518a015eeda423777322b2eca0179069c1928595dc99325cf1bd4d8",
}

POOL_RICH_SHA256 = "6c8cdb65a30ab293e620913d24e1344663c18e2f0befcf49a534be5771dbabe0"

COMPREHENSIVE_EXCLUSIONS = [
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
    "apps/classifier/data/manual_eval_set_200.jsonl",
    "apps/classifier/data/manual_eval_set_balanced_2000.jsonl",
    "apps/classifier/data/al_labeled_1129_true.jsonl",
    "apps/classifier/data/manual_eval_set_1000.jsonl",
    "apps/classifier/data/edge_cases.jsonl",
    "apps/classifier/data/edge_cases_large.jsonl",
    "apps/classifier/data/gold_pilot/gold_pilot_blind.jsonl",
]


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def clean_infohash(ih: str) -> str:
    ih = ih.strip().lower()
    if ih.startswith(r"\x"):
        ih = ih[2:]
    return ih


def verify_v0_integrity():
    """Verify that Gold Pilot v0 files have not been modified."""
    v0_paths = {
        "manifest": "apps/classifier/data/gold_pilot/gold_pilot_manifest.json",
        "blind": "apps/classifier/data/gold_pilot/gold_pilot_blind.jsonl",
        "template": "apps/classifier/data/gold_pilot/gold_pilot_review_template.jsonl",
    }
    for k, p in v0_paths.items():
        if os.path.exists(p):
            current_sha = sha256_file(p)
            assert current_sha == V0_CHECKSUMS[k], f"CRITICAL: Gold Pilot v0 file {p} was modified!"


def generate_pilot_v1_id(infohash: str, index: int) -> str:
    """Generate a collision-safe deterministic opaque identifier: GP1-XXXXXXXXXXXXXXXX."""
    payload = f"{NAMESPACE_V1}:{infohash}:{index}".encode("utf-8")
    digest = hashlib.sha256(payload).hexdigest()[:16].upper()
    return f"GP1-{digest}"


def parse_representative_files(raw_files) -> list[str]:
    """Safely parse PostgreSQL files JSONB representation."""
    if not raw_files:
        return []
    parsed = []
    if isinstance(raw_files, list):
        for item in raw_files:
            if isinstance(item, str) and item.strip():
                parsed.append(item.strip())
            elif isinstance(item, dict):
                p = item.get("path")
                if isinstance(p, list):
                    parsed.append("/".join(str(x) for x in p if str(x).strip()))
                elif isinstance(p, str) and p.strip():
                    parsed.append(p.strip())
            elif isinstance(item, list):
                parsed.append("/".join(str(x) for x in item if str(x).strip()))
    # Limit path length to 200 chars and return valid non-empty
    return [p[:200] for p in parsed if p]


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


def load_comprehensive_exclusions(exclude_eval_1000: bool = False) -> tuple[set[str], set[str], set[str], dict[str, str]]:
    """Load exclusion sets across all historical datasets."""
    ex_hashes = set()
    ex_names = set()
    ex_families = set()
    checksums = {}

    for ef in COMPREHENSIVE_EXCLUSIONS:
        if not exclude_eval_1000 and ef.endswith("manual_eval_set_1000.jsonl"):
            continue  # manual_eval_set_1000 is the source for natural stratum
        if os.path.exists(ef):
            checksums[ef] = sha256_file(ef)
            with open(ef, "r", encoding="utf-8", errors="ignore") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        r = json.loads(line)
                        ih = clean_infohash(r.get("infohash", r.get("id", "")))
                        nm = r.get("name", "").strip()
                        if ih:
                            ex_hashes.add(ih)
                        if nm:
                            ex_names.add(normalize_full_name(nm))
                            ex_families.add(normalize_release_family(nm))
                    except Exception:
                        pass
    return ex_hashes, ex_names, ex_families, checksums


def select_natural_stratum(
    eval_file: str,
    ex_hashes: set[str],
    ex_names: set[str],
    ex_families: set[str],
    target_count: int = 200,
    seed: int = SEED,
) -> tuple[list[dict], dict[str, int]]:
    """Select 200 clean, single-file natural records."""
    with open(eval_file, "r", encoding="utf-8") as f:
        records = [json.loads(l) for l in f if l.strip()]

    rng = random.Random(seed)
    indices = list(range(len(records)))
    rng.shuffle(indices)

    selected = []
    seen_h, seen_n, seen_f = set(), set(), set()
    exclusion_stats = Counter()

    for idx in indices:
        r = records[idx]
        ih = clean_infohash(r.get("infohash", r.get("id", "")))
        nm = r.get("name", "").strip()
        fc = int(r.get("file_count", 0))
        sz = int(r.get("total_size_bytes", r.get("total_size", 0)))

        # Natural single-file requirement
        if fc != 1 or sz <= 0 or not nm:
            exclusion_stats["constraint_single_file_or_size"] += 1
            continue

        nn = normalize_full_name(nm)
        rf = normalize_release_family(nm)

        # Three-level exclusion check
        if ih in ex_hashes:
            exclusion_stats["historical_exact_hash_leak"] += 1
            continue
        if nn in ex_names:
            exclusion_stats["historical_normalized_name_leak"] += 1
            continue
        if rf in ex_families:
            exclusion_stats["historical_release_family_leak"] += 1
            continue

        # Internal deduplication
        if ih in seen_h:
            exclusion_stats["internal_duplicate_hash"] += 1
            continue
        if nn in seen_n:
            exclusion_stats["internal_duplicate_name"] += 1
            continue
        if rf in seen_f:
            exclusion_stats["internal_duplicate_release_family"] += 1
            continue

        seen_h.add(ih)
        seen_n.add(nn)
        seen_f.add(rf)

        r["__source_dataset__"] = eval_file
        r["__source_row_idx__"] = idx
        r["__stratum__"] = "natural"
        r["__retrieval_group__"] = "natural_random"
        selected.append(r)

        if len(selected) >= target_count:
            break

    return selected, dict(exclusion_stats)


def select_diagnostic_stratum(
    pool_file: str,
    ex_hashes: set[str],
    ex_names: set[str],
    ex_families: set[str],
    natural_hashes: set[str],
    natural_names: set[str],
    natural_families: set[str],
) -> tuple[list[dict], dict[str, int], dict[str, int], int]:
    """Select 100 rich multi-file diagnostic records across 6 quotas."""
    group_quotas = {
        "anime_tv_boundary": 20,
        "app_game_boundary": 20,
        "game_other_boundary": 15,
        "movie_doc_boundary": 15,
        "rare_music_doc": 15,
        "difficult_other_mixed": 15,
    }

    group_collected = {g: [] for g in group_quotas}
    all_exclusions_h = set(ex_hashes) | set(natural_hashes)
    all_exclusions_n = set(ex_names) | set(natural_names)
    all_exclusions_f = set(ex_families) | set(natural_families)

    pool_records = []
    with open(pool_file, "r", encoding="utf-8") as f:
        for i, line in enumerate(f):
            if line.strip():
                r = json.loads(line)
                r["__source_row_idx__"] = i
                pool_records.append(r)

    seen_h, seen_n, seen_f = set(), set(), set()
    exclusion_stats = Counter()
    multi_group_count = 0

    for r in pool_records:
        ih = clean_infohash(r["infohash"])
        nm = r["name"].strip()
        fc = int(r["file_count"])
        sz = int(r["total_size"])
        files = r.get("files", [])

        if fc <= 1 or sz <= 0 or not nm or not files:
            exclusion_stats["invalid_metadata"] += 1
            continue

        nn = normalize_full_name(nm)
        rf = normalize_release_family(nm)

        # Exclusions
        if ih in all_exclusions_h:
            exclusion_stats["exact_hash_leak"] += 1
            continue
        if nn in all_exclusions_n:
            exclusion_stats["normalized_name_leak"] += 1
            continue
        if rf in all_exclusions_f:
            exclusion_stats["release_family_leak"] += 1
            continue

        if ih in seen_h or nn in seen_n or rf in seen_f:
            exclusion_stats["internal_duplicate"] += 1
            continue

        paths = parse_representative_files(files)
        if not paths:
            exclusion_stats["no_parseable_paths"] += 1
            continue

        all_text = (nm + " " + " ".join(paths[:10])).lower()

        # Identify all matching groups
        matching_groups = []
        if re.search(r"\[subs|\[erai|\[gjm|\[horrible|anime|s\d{1,2}e\d{1,3}|season|episode|ova|ona|tvrip", all_text):
            matching_groups.append("anime_tv_boundary")
        if re.search(r"setup\.exe|portable|patch|keygen|crack|repack|fitgirl|dodi|iso|dmg|vst|plugin|utility|tools|suite|office|adobe|windows|driver", all_text):
            matching_groups.append("app_game_boundary")
        if re.search(r"rom|nsp|xci|pkg|vpk|cia|gog|steam|switch|ps4|ps5|xbox|nintendo|emulator|soundtrack|dlc|trainer", all_text):
            matching_groups.append("game_other_boundary")
        if re.search(r"documentary|docuseries|bbc|pbs|nova|natgeo|discovery|history|biography|remux|bluray|2160p|1080p|dvdrip|web-dl", all_text):
            matching_groups.append("movie_doc_boundary")
        if re.search(r"flac|mp3|discography|album|soundtrack|ost|vinyl|lossless|alac|320kbps|ep|single|remastered", all_text):
            matching_groups.append("rare_music_doc")
        if re.search(r"pdf|epub|cbz|cbr|manga|tutorial|course|udemy|coursera|xxx|porn|jav|doujinshi|archive|collection|bundle|backup|\.rar|\.7z|\.zip", all_text):
            matching_groups.append("difficult_other_mixed")

        if len(matching_groups) > 1:
            multi_group_count += 1

        # Assign to first unsaturated primary group
        assigned = None
        for g in matching_groups:
            if len(group_collected[g]) < group_quotas[g]:
                assigned = g
                break

        if assigned:
            seen_h.add(ih)
            seen_n.add(nn)
            seen_f.add(rf)
            r["__source_dataset__"] = pool_file
            r["__stratum__"] = "diagnostic"
            r["__primary_group__"] = assigned
            r["__secondary_groups__"] = [g for g in matching_groups if g != assigned]
            r["__parsed_paths__"] = paths
            group_collected[assigned].append(r)

        if sum(len(v) for v in group_collected.values()) >= 100:
            break

    selected = []
    for g in group_quotas.keys():
        selected.extend(group_collected[g])

    group_counts = {g: len(v) for g, v in group_collected.items()}
    return selected, group_counts, dict(exclusion_stats), multi_group_count


def finalize_gold_pilot_v1(out_dir: str = "apps/classifier/data/gold_pilot_v1") -> dict:
    """Finalize Gold Pilot v1 with exactly 300 records."""
    verify_v0_integrity()
    out_path = Path(out_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    pool_rich_path = out_path / "candidate_pool_rich.jsonl"
    assert pool_rich_path.exists(), f"candidate_pool_rich.jsonl not found at {pool_rich_path}"
    assert sha256_file(str(pool_rich_path)) == POOL_RICH_SHA256, "Candidate pool SHA256 mismatch!"

    print("=================================================================")
    print("FINALIZING GOLD PILOT V1 (300 SAMPLES: 200 NATURAL + 100 DIAGNOSTIC)")
    print("=================================================================")

    # 1. Natural Stratum (200 records)
    print("\n1. Selecting Natural Stratum (200 records)...")
    ex_hashes, ex_names, ex_families, _ = load_comprehensive_exclusions(exclude_eval_1000=False)
    eval_1000_path = "apps/classifier/data/manual_eval_set_1000.jsonl"
    natural_samples, natural_exclusions = select_natural_stratum(
        eval_1000_path, ex_hashes, ex_names, ex_families, target_count=200, seed=SEED
    )
    assert len(natural_samples) == 200

    natural_hashes = set(clean_infohash(r["infohash"]) for r in natural_samples)
    natural_names = set(normalize_full_name(r["name"]) for r in natural_samples)
    natural_families = set(normalize_release_family(r["name"]) for r in natural_samples)

    # 2. Diagnostic Stratum (100 records)
    print("\n2. Selecting Diagnostic Stratum from Rich Candidate Pool (100 records)...")
    diag_ex_hashes, diag_ex_names, diag_ex_families, _ = load_comprehensive_exclusions(exclude_eval_1000=True)
    diagnostic_samples, diag_quotas, diag_exclusions, multi_group_count = select_diagnostic_stratum(
        str(pool_rich_path),
        diag_ex_hashes,
        diag_ex_names,
        diag_ex_families,
        natural_hashes,
        natural_names,
        natural_families,
    )
    assert len(diagnostic_samples) == 100

    all_pilot_records = natural_samples + diagnostic_samples
    assert len(all_pilot_records) == 300

    # 3. Generate Blind, Template, and Manifest records
    blind_records = []
    template_records = []
    manifest_records = []
    pilot_ids = set()

    for i, r in enumerate(all_pilot_records):
        ih = clean_infohash(r["infohash"])
        name = r["name"].strip()
        fc = int(r["file_count"])
        size = int(r.get("total_size", r.get("total_size_bytes", 0)))
        stratum = r["__stratum__"]

        pilot_id = generate_pilot_v1_id(ih, i)
        assert pilot_id not in pilot_ids
        pilot_ids.add(pilot_id)

        if stratum == "natural":
            metadata_mode = "sparse_single_file"
            files = []
            ext_summary = {}
            retrieval_group = "natural_random"
            secondary_groups = []
        else:
            metadata_mode = "rich_multi_file"
            parsed_paths = r["__parsed_paths__"]
            files = parsed_paths[:20]  # Max 20 representative paths
            ext_summary = extract_extension_summary(files)
            retrieval_group = r["__primary_group__"]
            secondary_groups = r["__secondary_groups__"]

        # Blind record (Reviewer visible only)
        blind_records.append({
            "pilot_id": pilot_id,
            "name": name,
            "file_count": fc,
            "total_size_bytes": size,
            "files": files,
            "extension_summary": ext_summary,
            "metadata_mode": metadata_mode,
        })

        # Template record (Null annotation fields)
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

        # Manifest record (Internal audit tracking)
        manifest_records.append({
            "pilot_id": pilot_id,
            "infohash": ih,
            "stratum": stratum,
            "metadata_mode": metadata_mode,
            "retrieval_group": retrieval_group,
            "secondary_groups": secondary_groups,
            "source_dataset": r["__source_dataset__"],
            "source_row_idx": r["__source_row_idx__"],
            "normalized_name_hash": hashlib.sha256(normalize_full_name(name).encode()).hexdigest(),
            "release_family_hash": hashlib.sha256(normalize_release_family(name).encode()).hexdigest(),
        })

    # Write files
    blind_path = out_path / "gold_pilot_v1_blind.jsonl"
    with open(blind_path, "w", encoding="utf-8") as f:
        for r in blind_records:
            f.write(json.dumps(r) + "\n")

    template_path = out_path / "gold_pilot_v1_review_template.jsonl"
    with open(template_path, "w", encoding="utf-8") as f:
        for r in template_records:
            f.write(json.dumps(r) + "\n")

    manifest = {
        "pilot_version": "1.0",
        "generation_timestamp": datetime.now(timezone.utc).isoformat(),
        "random_seed": SEED,
        "sample_counts": {
            "total": 300,
            "natural_stratum": 200,
            "diagnostic_stratum": 100,
        },
        "diagnostic_quotas": diag_quotas,
        "multi_group_candidates_count": multi_group_count,
        "exclusions": {
            "natural_stratum": natural_exclusions,
            "diagnostic_stratum": diag_exclusions,
        },
        "checksums": {
            "candidate_pool_rich_sha256": POOL_RICH_SHA256,
            "gold_pilot_v1_blind_sha256": sha256_file(str(blind_path)),
            "gold_pilot_v1_review_template_sha256": sha256_file(str(template_path)),
        },
        "records": manifest_records,
    }

    manifest_path = out_path / "gold_pilot_v1_manifest.json"
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)

    gen_report = {
        "generation_timestamp": datetime.now(timezone.utc).isoformat(),
        "total_records": 300,
        "natural_records": 200,
        "diagnostic_records": 100,
        "diagnostic_quotas": diag_quotas,
        "multi_group_candidates_count": multi_group_count,
        "exclusions": {
            "natural_stratum": natural_exclusions,
            "diagnostic_stratum": diag_exclusions,
        },
        "v0_checksums_verified": True,
        "pool_rich_checksum_verified": True,
        "output_files": {
            "manifest": str(manifest_path),
            "blind": str(blind_path),
            "template": str(template_path),
        },
    }
    report_path = out_path / "gold_pilot_v1_generation_report.json"
    with open(report_path, "w", encoding="utf-8") as f:
        json.dump(gen_report, f, indent=2)

    verify_v0_integrity()
    print("✅ All v0 checksums verified unchanged.")
    print("✅ Successfully generated Gold Pilot v1!")
    return manifest


if __name__ == "__main__":
    finalize_gold_pilot_v1()
