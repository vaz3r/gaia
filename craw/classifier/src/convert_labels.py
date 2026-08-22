#!/usr/bin/env python3
"""Convert labeled data from git classify/ format to JSONL."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def convert(sample_path: str, label_map_path: str, output_path: str):
    with open(sample_path, encoding="utf-8") as f:
        sample = json.load(f)

    label_map = {}
    if label_map_path:
        ns = {}
        exec(Path(label_map_path).read_text(), ns)
        raw = ns.get("L", {})
        for idx_str, (cat, keep) in raw.items():
            label_map[int(idx_str)] = (cat, keep)

    with open(output_path, "w", encoding="utf-8") as out_f:
        for i, row in enumerate(sample):
            infohash = row.get("id", "")
            name = row.get("name", "")
            file_count = row.get("file_count", 0)
            total_size = row.get("total_size", 0)
            top_dirs = row.get("top_dirs", []) or []

            label_cat = row.get("label_category", "")
            label_keep = row.get("label_keep", True)

            if not label_cat and i in label_map:
                label_cat, label_keep = label_map[i]

            if not label_cat:
                continue

            entry = {
                "infohash": infohash,
                "name": name,
                "file_count": file_count,
                "total_size": total_size,
                "top_dirs": top_dirs,
                "label_category": label_cat,
                "label_keep": label_keep,
            }
            out_f.write(json.dumps(entry) + "\n")

    print(f"Converted {i + 1} samples to {output_path}")


def main():
    parser = argparse.ArgumentParser(description="Convert labeled data to JSONL")
    parser.add_argument("--sample", default="data/torrent_sample.jsonl",
                        help="Source sample file (JSON array or JSONL)")
    parser.add_argument("--label-map", default="",
                        help="Path to label_map.py from git classify/")
    parser.add_argument("--output", default="data/labeled.jsonl",
                        help="Output JSONL file")
    args = parser.parse_args()

    sample_path = args.sample
    if sample_path.endswith(".jsonl"):
        lines = Path(sample_path).read_text().strip().split("\n")
        data = [json.loads(l) for l in lines if l.strip()]
        tmp = sample_path + ".tmp.json"
        with open(tmp, "w") as f:
            json.dump(data, f)
        sample_path = tmp

    convert(sample_path, args.label_map, args.output)


if __name__ == "__main__":
    main()
