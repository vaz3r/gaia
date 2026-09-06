#!/usr/bin/env python3
"""
Merge all labeled batches from labeling/labeled/ into a single training file.
Validates format, deduplicates by infohash, and outputs to data/human_labeled_v2/merged.jsonl.
"""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

VALID_CATEGORIES = {
    "Anime", "Applications", "Documentaries", "Games",
    "Movies", "Music", "Other", "Television",
}

LABELED_DIR = Path("labeling/labeled")
OUTPUT_FILE = Path("data/human_labeled_v2/merged.jsonl")


def load_labeled_batch(filepath: Path) -> list[dict]:
    """Load and validate a labeled batch file."""
    with open(filepath, encoding="utf-8") as f:
        content = f.read().strip()

    # Strip markdown fences if present
    if content.startswith("```"):
        lines = content.split("\n")
        lines = [l for l in lines if not l.strip().startswith("```")]
        content = "\n".join(lines)

    try:
        data = json.loads(content)
    except json.JSONDecodeError as e:
        print(f"  [WARN] {filepath.name}: invalid JSON ({e})")
        return []

    if not isinstance(data, list):
        print(f"  [WARN] {filepath.name}: not a JSON array")
        return []

    valid = []
    for item in data:
        cat = item.get("label_category", "")
        if cat not in VALID_CATEGORIES:
            print(f"  [WARN] {filepath.name}: invalid category '{cat}' for infohash {item.get('infohash', '?')[:12]}...")
            continue
        valid.append(item)

    return valid


def main():
    all_items = []
    seen_ihs = set()
    skipped = 0

    for category_dir in sorted(LABELED_DIR.iterdir()):
        if not category_dir.is_dir():
            continue

        batch_files = sorted(category_dir.glob("batch_*_labeled.json"))
        if not batch_files:
            # Also check for non-_labeled suffix
            batch_files = sorted(category_dir.glob("batch_*.json"))

        print(f"\n{category_dir.name}/")
        for bf in batch_files:
            items = load_labeled_batch(bf)
            for item in items:
                ih = item.get("infohash", "")
                if ih in seen_ihs:
                    skipped += 1
                    continue
                seen_ihs.add(ih)

                # Normalize to training format
                all_items.append({
                    "infohash": ih,
                    "name": item.get("name", ""),
                    "file_count": item.get("file_count", 0),
                    "total_size_bytes": item.get("total_size_bytes", 0),
                    "top_dirs": item.get("top_dirs", []),
                    "label_category": item["label_category"],
                    "sample_weight": 1.0,
                    "is_pseudo": False,
                    "source": f"labeling/labeled/{category_dir.name}/{bf.name}",
                })

            print(f"  {bf.name}: {len(items)} items")

    # Write output
    OUTPUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(OUTPUT_FILE, "w", encoding="utf-8") as f:
        for item in all_items:
            f.write(json.dumps(item, ensure_ascii=False) + "\n")

    # Summary
    print(f"\n{'=' * 60}")
    print(f"Total labeled: {len(all_items)}")
    print(f"Duplicates skipped: {skipped}")
    print(f"Output: {OUTPUT_FILE}")

    cat_counts = Counter(item["label_category"] for item in all_items)
    print(f"\nCategory distribution:")
    for cat, cnt in sorted(cat_counts.items(), key=lambda x: -x[1]):
        print(f"  {cat:<18}: {cnt:>5} ({cnt/len(all_items)*100:.1f}%)")


if __name__ == "__main__":
    main()
