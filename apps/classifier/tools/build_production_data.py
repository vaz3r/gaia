#!/usr/bin/env python3
"""
Pool all candidate torrents from repo datasets, strictly isolating frozen eval sets,
normalizing class taxonomy, and generating stratified annotation sets.
"""
from __future__ import annotations

import glob
import json
import logging
import os
import re
from collections import Counter
from pathlib import Path

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("build_production_data")

ALLOWED_CLASSES = [
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
]

EVAL_PATHS = [
    "data/gold_pilot_v1/reference_eval_v1.jsonl",
    "data/manual_eval_set_balanced_2000.jsonl",
    "data/manual_eval_set_1000.jsonl",
    "data/baseline_v2/validation.jsonl",
    "data/baseline_v1/validation.jsonl",
]


def load_eval_infohashes() -> set[str]:
    eval_hashes = set()
    for ep in EVAL_PATHS:
        p = Path(ep)
        if p.exists():
            with open(p, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    try:
                        obj = json.loads(line)
                        ih = obj.get("infohash")
                        if ih:
                            eval_hashes.add(ih.lower())
                    except Exception:
                        pass
    logger.info("Loaded %d strictly frozen evaluation infohashes.", len(eval_hashes))
    return eval_hashes


def normalize_label(label: str | None) -> str | None:
    if not label:
        return None
    l = str(label).strip()
    if l in ALLOWED_CLASSES:
        return l
    l_lower = l.lower()
    if l_lower in ["porn", "xxx", "adult", "junk", "misc", "book", "books", "course", "courses"]:
        return "Other"
    if l_lower in ["movie", "films", "film"]:
        return "Movies"
    if l_lower in ["tv", "series", "show", "shows", "tele"]:
        return "Television"
    if l_lower in ["game", "rom", "roms"]:
        return "Games"
    if l_lower in ["app", "apps", "software", "programs"]:
        return "Applications"
    if l_lower in ["doc", "docs", "documentary"]:
        return "Documentaries"
    if l_lower in ["song", "songs", "audio", "flac"]:
        return "Music"
    return None


def pool_all_candidates():
    eval_hashes = load_eval_infohashes()
    candidates: dict[str, dict] = {}

    data_dir = Path("data")
    jsonl_files = sorted(
        list(data_dir.rglob("*.jsonl")) + list(data_dir.glob("*.jsonl"))
    )

    for p in jsonl_files:
        p_str = str(p)
        # Skip eval sets
        if any(ev in p_str for ev in ["reference_eval", "manual_eval_set", "validation.jsonl", "reviewer_a", "reviewer_b"]):
            continue

        try:
            with open(p, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not (line.startswith("{") and line.endswith("}")):
                        continue
                    try:
                        obj = json.loads(line)
                    except Exception:
                        continue

                    ih = obj.get("infohash")
                    if not ih:
                        continue
                    ih_clean = str(ih).lower()
                    if ih_clean in eval_hashes:
                        continue

                    name = str(obj.get("name", "")).strip()
                    if not name:
                        continue

                    files = obj.get("files", obj.get("top_dirs", [])) or []
                    if isinstance(files, list) and files and isinstance(files[0], dict):
                        files = ["/".join(str(x) for x in f.get("path", [])) if isinstance(f.get("path"), list) else str(f.get("path")) for f in files]
                    elif isinstance(files, list) and files and isinstance(files[0], list):
                        files = ["/".join(str(x) for x in f) for f in files]

                    file_count = int(obj.get("file_count", len(files) if isinstance(files, list) else 1))
                    total_size = int(obj.get("total_size", obj.get("total_size_bytes", 0)))

                    raw_label = obj.get("label_category", obj.get("TRUE_LABEL", None))
                    norm_label = normalize_label(raw_label)

                    if ih_clean not in candidates:
                        candidates[ih_clean] = {
                            "infohash": ih_clean,
                            "name": name,
                            "file_count": file_count,
                            "total_size_bytes": total_size,
                            "files": files if isinstance(files, list) else [],
                            "label_category": norm_label,
                            "sources": [p_str],
                        }
                    else:
                        cand = candidates[ih_clean]
                        if not cand["label_category"] and norm_label:
                            cand["label_category"] = norm_label
                        if len(files) > len(cand["files"]):
                            cand["files"] = files
                            cand["file_count"] = max(file_count, cand["file_count"])
                            cand["total_size_bytes"] = max(total_size, cand["total_size_bytes"])
                        cand["sources"].append(p_str)
        except Exception as e:
            logger.warning("Error reading %s: %s", p_str, e)

    logger.info("Total clean non-eval candidates pooled: %d", len(candidates))
    
    labeled = {k: v for k, v in candidates.items() if v["label_category"]}
    unlabeled = {k: v for k, v in candidates.items() if not v["label_category"]}
    
    logger.info("Labeled candidates: %d", len(labeled))
    logger.info("Unlabeled candidates: %d", len(unlabeled))
    logger.info("Class distribution in labeled pool: %s", dict(Counter(v["label_category"] for v in labeled.values())))

    out_dir = Path("data/production_pool")
    out_dir.mkdir(parents=True, exist_ok=True)

    with open(out_dir / "pooled_labeled.jsonl", "w", encoding="utf-8") as f:
        for item in labeled.values():
            f.write(json.dumps(item) + "\n")

    with open(out_dir / "pooled_unlabeled.jsonl", "w", encoding="utf-8") as f:
        for item in unlabeled.values():
            f.write(json.dumps(item) + "\n")

    logger.info("Saved pooled datasets to %s", out_dir)


if __name__ == "__main__":
    pool_all_candidates()
