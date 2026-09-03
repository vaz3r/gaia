#!/usr/bin/env python3
"""
Train MLP classifier for torrent metadata.

Uses TF-IDF on text + numeric features from raw metadata.
No regex — learns patterns from raw data via TF-IDF.

Usage:
    python src/train_mlp.py
    python src/train_mlp.py --data data/labeled_data/train.jsonl --config config/mlp.yaml
"""
from __future__ import annotations

import argparse
import json
import logging
import sys
from pathlib import Path

import joblib
import numpy as np
import pandas as pd
import yaml
from sklearn.compose import ColumnTransformer
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.metrics import classification_report, accuracy_score, precision_recall_fscore_support
from sklearn.model_selection import train_test_split
from sklearn.neural_network import MLPClassifier
from sklearn.preprocessing import LabelEncoder, StandardScaler
from sklearn.pipeline import Pipeline

sys.path.insert(0, str(Path(__file__).parent))
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features, NUMERIC_FEATURE_NAMES

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

SEED = 42


def load_config(config_path: str | None) -> dict:
    if config_path and Path(config_path).exists():
        with open(config_path) as f:
            return yaml.safe_load(f)
    return {}


def load_data(path: str) -> list[dict]:
    items = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                items.append(json.loads(line))
    return items


def main():
    parser = argparse.ArgumentParser(description="Train MLP classifier")
    parser.add_argument("--data", default="data/labeled_data/train.jsonl")
    parser.add_argument("--config", default="config/mlp.yaml")
    parser.add_argument("--out_dir", default="data/models/mlp")
    args = parser.parse_args()

    config = load_config(args.config)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    # Load data
    logger.info("Loading dataset from %s...", args.data)
    items = load_data(args.data)
    logger.info("Total samples: %d", len(items))

    # Build texts and extract features
    texts = [build_input_text(item, config) for item in items]
    numeric_features = [extract_numeric_features(item) for item in items]
    labels = [item.get("label_category", "") for item in items]

    # Create DataFrame for sklearn pipeline
    df = pd.DataFrame(numeric_features)
    df["text_features"] = texts

    # Label encoding
    le = LabelEncoder()
    y = le.fit_transform(labels)
    classes = list(le.classes_)
    logger.info("Classes: %d — %s", len(classes), classes)

    # Split
    X_train, X_test, y_train, y_test = train_test_split(
        df, y, test_size=0.1, random_state=SEED, stratify=y
    )
    logger.info("Train: %d | Test: %d", len(X_train), len(X_test))

    # Build preprocessing pipeline
    tfidf_cfg = config.get("tfidf", {})
    mlp_cfg = config.get("mlp", {})

    num_cols = NUMERIC_FEATURE_NAMES

    preprocessor = ColumnTransformer(
        transformers=[
            (
                "word_text",
                TfidfVectorizer(
                    analyzer="word",
                    ngram_range=tuple(tfidf_cfg.get("word_ngram_range", [1, 2])),
                    min_df=tfidf_cfg.get("word_min_df", 2),
                    max_features=tfidf_cfg.get("word_max_features", 12000),
                    sublinear_tf=True,
                ),
                "text_features",
            ),
            (
                "char_text",
                TfidfVectorizer(
                    analyzer="char_wb",
                    ngram_range=tuple(tfidf_cfg.get("char_ngram_range", [3, 5])),
                    min_df=tfidf_cfg.get("char_min_df", 3),
                    max_features=tfidf_cfg.get("char_max_features", 15000),
                    sublinear_tf=True,
                ),
                "text_features",
            ),
            ("num", StandardScaler(), num_cols),
        ]
    )

    clf = Pipeline(
        steps=[
            ("preprocessor", preprocessor),
            (
                "classifier",
                MLPClassifier(
                    hidden_layer_sizes=tuple(mlp_cfg.get("hidden_layer_sizes", [128, 32])),
                    activation=mlp_cfg.get("activation", "relu"),
                    solver=mlp_cfg.get("solver", "adam"),
                    alpha=mlp_cfg.get("alpha", 0.001),
                    batch_size=mlp_cfg.get("batch_size", 128),
                    max_iter=mlp_cfg.get("max_iter", 300),
                    early_stopping=mlp_cfg.get("early_stopping", True),
                    random_state=SEED,
                ),
            ),
        ]
    )

    # Train
    logger.info("Training MLP...")
    clf.fit(X_train, y_train)

    # Evaluate
    y_pred = clf.predict(X_test)
    acc = accuracy_score(y_test, y_pred)
    p, r, f1, _ = precision_recall_fscore_support(y_test, y_pred, average="macro", zero_division=0)

    logger.info("=" * 60)
    logger.info("Accuracy:  %.3f (%d/%d)", acc, int(acc * len(y_test)), len(y_test))
    logger.info("Macro F1:  %.3f", f1)
    logger.info("=" * 60)

    print("\n--- Classification Report ---")
    print(classification_report(y_test, y_pred, target_names=classes, digits=3))

    # Save
    model_path = out_dir / "torrent_classifier.joblib"
    joblib.dump({"model": clf, "label_encoder": le}, model_path)
    logger.info("Model saved to: %s", model_path)

    # Save config used
    config_path = out_dir / "config_used.yaml"
    with open(config_path, "w") as f:
        yaml.dump(config, f)
    logger.info("Config saved to: %s", config_path)


if __name__ == "__main__":
    main()
