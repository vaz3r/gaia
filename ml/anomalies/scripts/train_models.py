#!/usr/bin/env python3
import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import logging
from src.pipeline.train import train_all_models

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")

def main():
    print("=== Training Crawler Operations Anomaly Models ===")
    results = train_all_models()
    print("\n=== Training Summary ===")
    print(f"Dataset Size: {results['dataset_size']} hours")
    print(f"Flagged Anomalies (score >= 0.70): {results['anomalies_detected']}")
    for model_name, path in results["models"].items():
        print(f"  - {model_name}: {path}")
    print("\n[SUCCESS] All models trained and saved to models_storage/.")

if __name__ == "__main__":
    main()
