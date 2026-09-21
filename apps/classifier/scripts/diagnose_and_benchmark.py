#!/usr/bin/env python3
"""
Standalone Diagnostic & Fast Benchmark Tool for Gaia Torrent Classifier.
Evaluates on a frozen 5,000 holdout dataset in <10 seconds.
Identifies exact confusion pairs, failure cases, and review queue flags.
"""

import sys
import os
import json
import time
from pathlib import Path
from collections import Counter, defaultdict

import numpy as np
from sklearn.metrics import classification_report, confusion_matrix
from sklearn.model_selection import train_test_split

SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(SRC_DIR))

import db
from classifier_service import TorrentClassifierService
from feature_extractor import explain_features

HOLDOUT_FILE = Path(__file__).parent.parent / "data" / "holdout_5000.json"


def ensure_frozen_holdout():
    """Ensure a deterministic 5,000 holdout set is cached on disk."""
    if HOLDOUT_FILE.exists():
        with open(HOLDOUT_FILE, "r", encoding="utf-8") as f:
            records = json.load(f)
        return records

    print("[*] Fetching training data to create frozen holdout set...", flush=True)
    all_records = db.fetch_training_data(limit=60000)
    labels = [r["label_category"] for r in all_records]

    _, holdout = train_test_split(
        all_records,
        test_size=5000,
        random_state=42,
        stratify=labels
    )

    HOLDOUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(HOLDOUT_FILE, "w", encoding="utf-8") as f:
        json.dump(holdout, f)

    print(f"[*] Saved {len(holdout)} frozen holdout records to {HOLDOUT_FILE}", flush=True)
    return holdout


def run_benchmark():
    holdout = ensure_frozen_holdout()
    print("=" * 80, flush=True)
    print(f"GAIA CLASSIFIER FROZEN BENCHMARK ({len(holdout):,} records)", flush=True)
    print("=" * 80, flush=True)

    svc = TorrentClassifierService.get_instance()
    model_name = svc.model_path.name if svc.model_path else "unknown"
    version = svc.active_info.get("version", "unknown")
    print(f"Active Model: {version} ({model_name})\n", flush=True)

    t0 = time.time()
    batch_size = 500
    all_results = []
    for i in range(0, len(holdout), batch_size):
        chunk = holdout[i:i + batch_size]
        res = svc.classify_batch(chunk)
        all_results.extend(res)

    elapsed = time.time() - t0
    throughput = len(holdout) / max(elapsed, 0.001)

    # Metrics
    classes = svc.classes
    y_true = [r["label_category"] for r in holdout]
    y_pred = [r["predicted_category"] for r in all_results]
    confs = [r["confidence"] for r in all_results]
    needs_review = [r.get("needs_review", False) for r in all_results]

    correct_count = sum(1 for yt, yp in zip(y_true, y_pred) if yt == yp)
    total = len(y_true)
    accuracy = correct_count / total

    # Auto-accepted metrics
    acc_indices = [i for i, rev in enumerate(needs_review) if not rev]
    acc_count = len(acc_indices)
    acc_rate = acc_count / total
    acc_correct = sum(1 for idx in acc_indices if y_true[idx] == y_pred[idx])
    acc_precision = acc_correct / max(acc_count, 1)

    print("-" * 80)
    print(f"EXECUTION SPEED:       {elapsed:.3f}s ({throughput:,.1f} items/sec)")
    print(f"OVERALL ACCURACY:      {accuracy * 100:.2f}% ({correct_count}/{total})")
    print(f"AUTO-ACCEPT RATE:      {acc_rate * 100:.2f}% ({acc_count}/{total})")
    print(f"AUTO-ACCEPT PRECISION: {acc_precision * 100:.2f}% ({acc_correct}/{acc_count})")
    print(f"FLAGGED FOR REVIEW:    {(1 - acc_rate) * 100:.2f}% ({total - acc_count}/{total})")
    print("-" * 80)

    # Classification report
    all_labels = sorted(list(set(y_true) | set(y_pred)))
    print("\nPER-CLASS CLASSIFICATION REPORT:")
    print(classification_report(y_true, y_pred, labels=all_labels, target_names=all_labels, digits=2, zero_division=0))

    # Confusion matrix - top misclassifications
    cm = confusion_matrix(y_true, y_pred, labels=all_labels)
    misclass = []
    for i, true_cat in enumerate(all_labels):
        for j, pred_cat in enumerate(all_labels):
            if i != j and cm[i, j] > 0:
                misclass.append((cm[i, j], true_cat, pred_cat))

    misclass.sort(reverse=True, key=lambda x: x[0])
    print("\nTOP 10 CONFUSION PAIRS (True Label -> Misclassified As):")
    for cnt, true_c, pred_c in misclass[:10]:
        print(f"  {cnt:4d}x | True: {true_c:<18} -> Predicted: {pred_c:<18}")

    # Inspect sample failure cases
    print("\nSAMPLE MISCLASSIFIED EXAMPLES:")
    failures = []
    for i, (item, res) in enumerate(zip(holdout, all_results)):
        yt = item["label_category"]
        yp = res["predicted_category"]
        if yt != yp:
            failures.append((item, res, yt, yp))

    for item, res, yt, yp in failures[:8]:
        print(f"  - Title: '{item['name'][:65]}'")
        print(f"    True: [{yt}] | Pred: [{yp}] | Conf: {res['confidence']:.2f} | Margin: {res['margin']:.2f} | Review: {res['needs_review']}")
        raw_files = item.get("files") or []
        if isinstance(raw_files, list) and raw_files:
            file_names = [f.get("path") if isinstance(f, dict) else str(f) for f in raw_files[:3]]
            print(f"    Files ({len(raw_files)}): {file_names}")
        print()

    print("=" * 80)


if __name__ == "__main__":
    run_benchmark()
