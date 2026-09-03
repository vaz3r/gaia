#!/usr/bin/env python3
"""
Pseudo-label unlabeled torrents using the trained MLP model.

Queries torrents without labels, runs inference, filters by confidence,
and exports high-confidence predictions as pseudo-labels for training.

Usage:
    python src/pseudo_label.py --count 10000 --threshold 0.95
"""
from __future__ import annotations

import argparse
import json
import logging
import os
import sys
import time
from collections import Counter
from pathlib import Path

import joblib
import numpy as np
import psycopg2
import psycopg2.extras

sys.path.insert(0, str(Path(__file__).parent))
from backends.mlp_backend import MLPBackend
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features

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

FETCH_SQL = """
SELECT
    encode(t.infohash, 'hex') AS infohash,
    t.name,
    t.file_count,
    t.total_size,
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
FROM torrents t
WHERE NOT EXISTS (SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash)
  AND t.name IS NOT NULL
  AND t.name != ''
ORDER BY random()
LIMIT %s
"""


def get_db():
    return psycopg2.connect(**DB_CONFIG)


def fetch_unlabeled(count: int) -> list[dict]:
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(FETCH_SQL, (count,))
            rows = cur.fetchall()
            logger.info("Fetched %d unlabeled torrents", len(rows))
    finally:
        conn.close()

    items = []
    for row in rows:
        item = {
            "infohash": row["infohash"],
            "name": row["name"] or "",
            "file_count": row["file_count"] or 0,
            "total_size_bytes": row["total_size"] or 0,
            "extensions": row["extensions"] or [],
            "top_folders": row["top_folders"] or [],
            "largest_files": row["largest_files"] or [],
        }
        items.append(item)
    return items


def main():
    parser = argparse.ArgumentParser(description="Pseudo-label unlabeled torrents with MLP")
    parser.add_argument("--count", type=int, default=20000, help="Number of unlabeled torrents to fetch")
    parser.add_argument("--threshold", type=float, default=0.95, help="Minimum confidence threshold")
    parser.add_argument("--output", default="data/labeled_data/pseudo_labels.jsonl", help="Output JSONL path")
    parser.add_argument("--model", default="data/models/mlp/torrent_classifier.joblib", help="Model path")
    parser.add_argument("--batch_size", type=int, default=512, help="Inference batch size")
    args = parser.parse_args()

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)

    # Load model
    backend = MLPBackend(args.model)

    # Fetch unlabeled torrents
    logger.info("Fetching %d unlabeled torrents...", args.count)
    items = fetch_unlabeled(args.count)
    logger.info("Got %d torrents", len(items))

    # Build text and numeric features
    logger.info("Running inference (threshold=%.2f)...", args.threshold)
    texts = [build_input_text(item) for item in items]
    numeric_features = [extract_numeric_features(item) for item in items]

    # Run inference
    start_time = time.time()
    probs, preds = backend.predict(texts, numeric_features)
    elapsed = time.time() - start_time
    logger.info("Inference completed in %.1fs", elapsed)

    # Filter by confidence
    max_probs = probs.max(axis=-1)
    pred_labels = backend.label_encoder.inverse_transform(preds)

    kept = 0
    with open(output, "w", encoding="utf-8") as f:
        for i, item in enumerate(items):
            conf = float(max_probs[i])
            if conf >= args.threshold:
                record = {
                    "infohash": item["infohash"],
                    "name": item["name"],
                    "file_count": item["file_count"],
                    "total_size_bytes": item["total_size_bytes"],
                    "extensions": item["extensions"],
                    "top_folders": item["top_folders"],
                    "largest_files": item["largest_files"],
                    "label_category": pred_labels[i],
                    "confidence": "high",
                    "reason": f"pseudo-label (conf={conf:.4f})",
                    "source": "mlp_pseudo",
                }
                f.write(json.dumps(record, ensure_ascii=False) + "\n")
                kept += 1

    logger.info("Kept %d / %d predictions (%.1f%%) with confidence >= %.2f",
                kept, len(items), kept / len(items) * 100, args.threshold)

    # Distribution
    kept_labels = [pred_labels[i] for i in range(len(items)) if max_probs[i] >= args.threshold]
    dist = Counter(kept_labels)
    logger.info("Category distribution:")
    for cat, cnt in sorted(dist.items(), key=lambda x: -x[1]):
        logger.info("  %-18s: %5d (%.1f%%)", cat, cnt, cnt / kept * 100)

    logger.info("Wrote: %s", output)


if __name__ == "__main__":
    main()
