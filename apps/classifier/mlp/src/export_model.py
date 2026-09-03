#!/usr/bin/env python3
"""Export trained MLP model to joblib format."""

from __future__ import annotations

import argparse
import logging
from pathlib import Path

import joblib

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)


def export_model(model_path: str, output_path: str):
    """Export model to joblib format for production use."""
    data = joblib.load(model_path)
    model = data["model"]
    le = data["label_encoder"]

    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)

    # Save model + encoder
    joblib.dump({"model": model, "label_encoder": le}, output)
    logger.info("Exported model to: %s", output)
    logger.info("Model size: %.1f MB", output.stat().st_size / 1024 / 1024)

    return output


def main():
    parser = argparse.ArgumentParser(description="Export MLP model")
    parser.add_argument("--model", default="data/models/mlp/torrent_classifier.joblib")
    parser.add_argument("--output", default="data/models/mlp/torrent_classifier_export.joblib")
    args = parser.parse_args()

    export_model(args.model, args.output)


if __name__ == "__main__":
    main()
