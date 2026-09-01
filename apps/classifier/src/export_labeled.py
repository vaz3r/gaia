#!/usr/bin/env python3
"""
Export labeled data from PostgreSQL to training JSONL.

Queries labeled_results (written by MCP server) joined with torrents
for full metadata, then splits into train (90%) and test (10%) sets.

Usage:
    python src/export_labeled.py
    python src/export_labeled.py --output data/gemini_labeled/merged.jsonl
    python src/export_labeled.py --min-confidence medium
"""
from __future__ import annotations

import argparse
import json
import logging
import os
import sys
from collections import Counter
from pathlib import Path

import psycopg2
import psycopg2.extras

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

DB_CONFIG = {
    "host": os.getenv("DB_HOST", "workspace-production"),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
    "connect_timeout": 10,
}

CONFIDENCE_ORDER = {"high": 3, "medium": 2, "low": 1}

EXPORT_SQL = """
SELECT
    encode(t.infohash, 'hex') AS infohash,
    t.name,
    t.file_count,
    t.total_size,
    lr.label_category,
    lr.confidence,
    lr.reason,
    lr.source,
    CASE
        WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
            (
                SELECT array_agg(DISTINCT ext)
                FROM (
                    SELECT
                        CASE
                            WHEN jsonb_array_length(elem->'path') > 0 THEN
                                lower(split_part(elem->'path'->>-1, '.', -1))
                            ELSE NULL
                        END AS ext
                    FROM jsonb_array_elements(t.files) AS elem
                ) sub
                WHERE ext IS NOT NULL AND ext != ''
                LIMIT 10
            )
        ELSE NULL
    END AS extensions,
    CASE
        WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
            (
                SELECT array_agg(DISTINCT folder)
                FROM (
                    SELECT
                        CASE
                            WHEN jsonb_array_length(elem->'path') > 1 THEN
                                elem->'path'->>0
                            ELSE NULL
                        END AS folder
                    FROM jsonb_array_elements(t.files) AS elem
                ) sub
                WHERE folder IS NOT NULL
                LIMIT 10
            )
        ELSE NULL
    END AS top_folders,
    CASE
        WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
            (
                SELECT jsonb_agg(jsonb_build_object(
                    'name', sub.elem->'path'->>-1,
                    'size', sub.elem->'length'
                ))
                FROM (
                    SELECT elem
                    FROM jsonb_array_elements(t.files) AS elem
                    ORDER BY (elem->'length')::bigint DESC
                    LIMIT 3
                ) sub
            )
        ELSE NULL
    END AS largest_files
FROM labeled_results lr
JOIN torrents t ON t.infohash = lr.infohash
ORDER BY lr.label_category, random()
"""


def get_db():
    return psycopg2.connect(**DB_CONFIG)


def export_labeled(min_confidence: str = "low", output_path: str = "data/gemini_labeled/merged.jsonl"):
    min_conf = CONFIDENCE_ORDER.get(min_confidence, 1)
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)

    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(EXPORT_SQL)
            rows = cur.fetchall()
            logger.info(f"Fetched {len(rows)} labeled torrents from PostgreSQL")
    finally:
        conn.close()

    all_items = []
    for row in rows:
        conf = row["confidence"] or "medium"
        if CONFIDENCE_ORDER.get(conf, 0) < min_conf:
            continue

        extensions = row["extensions"] or []
        top_folders = row["top_folders"] or []
        largest_files = row["largest_files"] or []

        item = {
            "infohash": row["infohash"],
            "name": row["name"] or "",
            "file_count": row["file_count"] or 0,
            "total_size_bytes": row["total_size"] or 0,
            "extensions": extensions,
            "top_folders": top_folders,
            "largest_files": largest_files,
            "label_category": row["label_category"],
            "confidence": conf,
            "reason": row["reason"] or "",
            "source": row["source"] or "gemini_spark",
        }
        all_items.append(item)

    logger.info(f"Exported {len(all_items)} items (min confidence: {min_confidence})")

    # Category distribution
    cat_counts = Counter(item["label_category"] for item in all_items)
    logger.info("Category distribution:")
    for cat, cnt in sorted(cat_counts.items(), key=lambda x: -x[1]):
        logger.info(f"  {cat:<18}: {cnt:>5} ({cnt/len(all_items)*100:.1f}%)")

    # Write merged file
    with open(output, "w", encoding="utf-8") as f:
        for item in all_items:
            f.write(json.dumps(item, ensure_ascii=False) + "\n")
    logger.info(f"Wrote: {output}")

    return all_items


def split_train_test(items: list[dict], output_dir: str, test_ratio: float = 0.10):
    """Stratified 90/10 train/test split."""
    from collections import defaultdict
    import random

    by_cat = defaultdict(list)
    for item in items:
        by_cat[item["label_category"]].append(item)

    train, test = [], []
    rng = random.Random(42)

    for cat, cat_items in by_cat.items():
        rng.shuffle(cat_items)
        n_test = max(1, int(len(cat_items) * test_ratio))
        test.extend(cat_items[:n_test])
        train.extend(cat_items[n_test:])

    rng.shuffle(train)
    rng.shuffle(test)

    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)

    train_path = out / "train.jsonl"
    test_path = out / "test.jsonl"

    with open(train_path, "w", encoding="utf-8") as f:
        for item in train:
            f.write(json.dumps(item, ensure_ascii=False) + "\n")

    with open(test_path, "w", encoding="utf-8") as f:
        for item in test:
            f.write(json.dumps(item, ensure_ascii=False) + "\n")

    logger.info(f"Train: {len(train)} items -> {train_path}")
    logger.info(f"Test:  {len(test)} items -> {test_path}")

    # Distribution
    for split_name, split_items in [("train", train), ("test", test)]:
        cats = Counter(i["label_category"] for i in split_items)
        logger.info(f"  {split_name}: " + ", ".join(f"{c}={n}" for c, n in sorted(cats.items())))


def main():
    parser = argparse.ArgumentParser(description="Export labeled data from PostgreSQL")
    parser.add_argument("--output", default="data/gemini_labeled/merged.jsonl")
    parser.add_argument("--min-confidence", default="low", choices=["low", "medium", "high"])
    parser.add_argument("--split", action="store_true", help="Also produce train/test split")
    args = parser.parse_args()

    items = export_labeled(args.min_confidence, args.output)

    if args.split and items:
        split_train_test(items, str(Path(args.output).parent))


if __name__ == "__main__":
    main()
