#!/usr/bin/env python3
"""MLP inference backend for torrent classification."""

from __future__ import annotations

import logging
from pathlib import Path

import joblib
import numpy as np

logger = logging.getLogger(__name__)


class MLPBackend:
    """Batch inference using sklearn MLP + TF-IDF model."""

    def __init__(self, model_path: str = "data/models/mlp/torrent_classifier.joblib"):
        logger.info("Loading MLP model from %s...", model_path)
        data = joblib.load(model_path)
        self.model = data["model"]
        self.label_encoder = data["label_encoder"]
        self.classes = self.label_encoder.classes_
        logger.info("Loaded MLP model with %d classes", len(self.classes))

    def predict(self, texts: list[str], numeric_features: list[dict]) -> tuple[np.ndarray, np.ndarray]:
        """Classify a batch of texts with numeric features.

        Returns:
            (probs, predictions) where probs is (N, num_classes) float64
            and predictions is (N,) int64 of class indices.
        """
        import pandas as pd
        from core.feature_extractor import NUMERIC_FEATURE_NAMES

        # Build DataFrame with text and numeric features
        df = pd.DataFrame(numeric_features)
        df["text_features"] = texts

        # Predict probabilities
        probs = self.model.predict_proba(df)
        predictions = probs.argmax(axis=-1)

        return probs, predictions

    def predict_labels(self, texts: list[str], numeric_features: list[dict]) -> tuple[list[str], np.ndarray]:
        """Classify and return human-readable labels.

        Returns:
            (labels, confidences) where labels is list of str
            and confidences is (N,) float64.
        """
        probs, preds = self.predict(texts, numeric_features)
        labels = self.label_encoder.inverse_transform(preds)
        confidences = probs.max(axis=-1)
        return list(labels), confidences
