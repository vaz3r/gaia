#!/usr/bin/env python3
"""Train linear classifier on embeddings for torrent classification."""

from __future__ import annotations

import argparse
import json
import logging
import sys
import time
from pathlib import Path

import joblib
import numpy as np
import yaml
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import classification_report
from sklearn.model_selection import train_test_split
from sklearn.preprocessing import LabelEncoder

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
logger = logging.getLogger(__name__)


def load_labeled_data(path: str) -> list[dict]:
    records = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                records.append(json.loads(line))
    return records


def main():
    parser = argparse.ArgumentParser(description="Train embedding classifier")
    parser.add_argument("--labels", required=True, help="Labeled JSONL file")
    parser.add_argument("--config", default="config/embedding.yaml", help="Embedding config")
    parser.add_argument("--output", default="data/models", help="Output directory")
    parser.add_argument("--test-size", type=float, default=0.2, help="Test split ratio")
    args = parser.parse_args()

    with open(args.config, encoding="utf-8") as f:
        config = yaml.safe_load(f)

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from src.core.text_builder import build_input_text
    from src.backends.embedding_backend import EmbeddingBackend

    records = load_labeled_data(args.labels)
    logger.info("Loaded %d labeled records", len(records))

    texts = [build_input_text(r, config) for r in records]
    labels = [r["label_category"] for r in records]

    le = LabelEncoder()
    y = le.fit_transform(labels)
    classes = list(le.classes_)
    logger.info("Classes: %s (counts: %s)",
                classes, [int(c) for c in np.bincount(y)])

    logger.info("Embedding texts...")
    t0 = time.time()
    emb_cfg = config.get("embedding", {})
    backend = EmbeddingBackend(emb_cfg.get("model_name", "Qwen/Qwen3-Embedding-0.6B"))
    X = backend.embed(texts, batch_size=emb_cfg.get("batch_size", 64))
    logger.info("Embedded in %.1fs, shape=%s", time.time() - t0, X.shape)

    cache_path = Path(args.output) / "embeddings_cache.npy"
    np.save(cache_path, X)
    logger.info("Cached embeddings to %s", cache_path)

    X_train, X_test, y_train, y_test = train_test_split(
        X, y, test_size=args.test_size, random_state=42, stratify=y
    )
    logger.info("Split: train=%d, test=%d", len(X_train), len(X_test))

    logger.info("Training LogisticRegression...")
    t0 = time.time()
    clf = LogisticRegression(
        max_iter=2000,
        solver="saga",
        C=1.0,
        class_weight="balanced",
        random_state=42,
    )
    clf.fit(X_train, y_train)
    logger.info("Trained in %.1fs", time.time() - t0)

    y_pred = clf.predict(X_test)
    report = classification_report(
        y_test, y_pred, target_names=classes, zero_division=0
    )
    print("\n=== Classification Report (test set) ===")
    print(report)

    out_dir = Path(args.output)
    out_dir.mkdir(parents=True, exist_ok=True)

    clf_path = out_dir / "logreg_category.joblib"
    joblib.dump(clf, clf_path)
    logger.info("Saved classifier to %s", clf_path)

    le_path = out_dir / "label_encoder.joblib"
    joblib.dump(le, le_path)
    logger.info("Saved label encoder to %s", le_path)

    report_path = out_dir / "classification_report.txt"
    report_path.write_text(report)
    logger.info("Saved report to %s", report_path)

    logger.info("Done.")


if __name__ == "__main__":
    main()
