#!/usr/bin/env python3
"""
Inference bridge for the MLP classifier.

Reads JSON torrent metadata from stdin, classifies it,
and writes JSON predictions to stdout.

Input (stdin):  {"name": "...", "file_count": 5, "total_size": 123456, ...}
Output (stdout): {"label": "Movies", "confidence": 0.95, "probabilities": {"Adult": 0.01, ...}}
"""
import json
import sys
import os
import traceback

# Add MLP src dir to path so we can import the classifier modules
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "src"))

from backends.mlp_backend import MLPBackend
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features

MODEL_PATH = os.path.join(
    os.path.dirname(__file__), "..", "..", "data", "models", "mlp_7k_human", "torrent_classifier.joblib"
)

_backend: MLPBackend | None = None


def get_backend() -> MLPBackend:
    global _backend
    if _backend is None:
        _backend = MLPBackend(MODEL_PATH)
    return _backend


def classify(torrent: dict) -> dict:
    """Classify a single torrent and return prediction result."""
    backend = get_backend()

    text = build_input_text(torrent)
    numeric = extract_numeric_features(torrent)

    probs, preds = backend.predict([text], [numeric])
    label = backend.label_encoder.inverse_transform(preds)[0]
    confidence = float(probs[0].max())

    # Build probability dict for all classes
    probabilities = {}
    for i, cls in enumerate(backend.classes):
        probabilities[cls] = float(probs[0][i])

    return {
        "label": label,
        "confidence": round(confidence, 4),
        "probabilities": {k: round(v, 4) for k, v in sorted(probabilities.items(), key=lambda x: -x[1])},
    }


def main():
    """Main loop: read JSON lines from stdin, write predictions to stdout."""
    # Pre-load model at startup
    get_backend()
    print(json.dumps({"status": "ready", "model": MODEL_PATH}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            torrent = json.loads(line)
            result = classify(torrent)
            result["status"] = "ok"
        except Exception as e:
            result = {"status": "error", "error": str(e), "traceback": traceback.format_exc()}

        print(json.dumps(result), flush=True)

    # stdin closed (pipe broken), exit cleanly
    sys.exit(0)


if __name__ == "__main__":
    main()
