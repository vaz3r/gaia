#!/usr/bin/env python3
"""Evaluate MLP model on test split."""

from __future__ import annotations

import json
import logging
import sys
from pathlib import Path

import joblib
import numpy as np
import pandas as pd
from sklearn.metrics import accuracy_score, precision_recall_fscore_support, confusion_matrix, classification_report

sys.path.insert(0, str(Path(__file__).parent))
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)


def load_benchmark(path: str) -> tuple[list[dict], list[str]]:
    torrents, labels = [], []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            torrents.append(row)
            labels.append(row.get("label_category", row.get("true_category", "")))
    return torrents, labels


def evaluate(model_path: str, benchmark_path: str):
    # Load model
    logger.info("Loading model from %s", model_path)
    data = joblib.load(model_path)
    model = data["model"]
    le = data["label_encoder"]
    classes = le.classes_

    # Load benchmark
    torrents, gold_labels = load_benchmark(benchmark_path)
    logger.info("Loaded %d samples from %s", len(torrents), benchmark_path)

    # Build features
    texts = [build_input_text(t) for t in torrents]
    numeric_features = [extract_numeric_features(t) for t in torrents]

    df = pd.DataFrame(numeric_features)
    df["text_features"] = texts

    # Predict
    preds = model.predict(df)
    pred_labels = [classes[p] for p in preds]

    # Metrics
    acc = accuracy_score(gold_labels, pred_labels)
    p, r, f1, _ = precision_recall_fscore_support(gold_labels, pred_labels, average="macro", zero_division=0)
    p_per, r_per, f1_per, support_per = precision_recall_fscore_support(
        gold_labels, pred_labels, average=None, zero_division=0, labels=sorted(set(gold_labels))
    )

    print(f"\n{'='*60}")
    print(f"MLP Model: {model_path}")
    print(f"Benchmark: {benchmark_path}")
    print(f"Samples:   {len(torrents)}")
    print(f"Accuracy:  {acc:.3f} ({int(acc*len(torrents))}/{len(torrents)})")
    print(f"Macro F1:  {f1:.3f}")
    print(f"{'='*60}")

    print(f"\n{'Category':<20} {'Precision':>9} {'Recall':>9} {'F1':>9} {'Support':>9}")
    print("-" * 56)
    for i, cat in enumerate(sorted(set(gold_labels))):
        print(f"{cat:<20} {p_per[i]:>9.3f} {r_per[i]:>9.3f} {f1_per[i]:>9.3f} {support_per[i]:>9}")

    # Confusion matrix
    cats = sorted(set(gold_labels))
    cm = confusion_matrix(gold_labels, pred_labels, labels=cats)
    print(f"\nConfusion matrix (gold=rows, pred=cols):")
    header = f"{'Gold\\Pred':<20}" + "".join(f"{c[:10]:>11}" for c in cats)
    print(header)
    for i, cat in enumerate(cats):
        row = f"{cat:<20}" + "".join(f"{cm[i][j]:>11}" for j in range(len(cats)))
        print(row)

    return acc, f1


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="data/models/mlp/torrent_classifier.joblib")
    parser.add_argument("--benchmark", default="data/labeled_data/test.jsonl")
    args = parser.parse_args()
    evaluate(args.model, args.benchmark)
