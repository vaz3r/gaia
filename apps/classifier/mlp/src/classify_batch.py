#!/usr/bin/env python3
"""
Batch classification CLI for torrent metadata using MLP + TF-IDF.

Accepts full torrent metadata (name, files, sizes, etc.)
and classifies using the trained MLP model.

Usage:
    python src/classify_batch.py --input torrents.jsonl --output classified.jsonl
    python src/classify_batch.py --input torrents.jsonl --model data/models/mlp/torrent_classifier.joblib
"""
from __future__ import annotations

import argparse
import json
import logging
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from backends.mlp_backend import MLPBackend
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)


def load_torrents(path: str, limit: int | None = None) -> list[dict]:
    torrents = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                torrents.append(json.loads(line))
            if limit and len(torrents) >= limit:
                break
    return torrents


def main():
    parser = argparse.ArgumentParser(description="Batch classify torrents with MLP")
    parser.add_argument("--input", required=True, help="Input JSONL file")
    parser.add_argument("--output", required=True, help="Output JSONL file")
    parser.add_argument("--model", default="data/models/mlp/torrent_classifier.joblib")
    parser.add_argument("--batch_size", type=int, default=512)
    parser.add_argument("--limit", type=int, default=None)
    args = parser.parse_args()

    # Load model
    backend = MLPBackend(args.model)

    # Load torrents
    torrents = load_torrents(args.input, args.limit)
    logger.info("Loaded %d torrents", len(torrents))

    # Process in batches
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)

    total = len(torrents)
    classified = 0
    start_time = time.time()

    with open(output, "w", encoding="utf-8") as f:
        for i in range(0, total, args.batch_size):
            batch = torrents[i : i + args.batch_size]

            texts = [build_input_text(t) for t in batch]
            numeric_features = [extract_numeric_features(t) for t in batch]

            labels, confidences = backend.predict_labels(texts, numeric_features)

            for j, torrent in enumerate(batch):
                record = {
                    **torrent,
                    "predicted_category": labels[j],
                    "confidence": float(confidences[j]),
                }
                f.write(json.dumps(record, ensure_ascii=False) + "\n")
                classified += 1

            elapsed = time.time() - start_time
            rate = classified / elapsed if elapsed > 0 else 0
            logger.info("Progress: %d/%d (%.0f samples/s)", classified, total, rate)

    elapsed = time.time() - start_time
    logger.info("Classified %d torrents in %.1fs (%.0f samples/s)", classified, elapsed, classified / elapsed)
    logger.info("Output: %s", output)


if __name__ == "__main__":
    main()
