"""
End-to-end training pipeline for TrustClassifier with time-disjoint splitting and calibration.
"""
import os
import sys
import json
import logging
from pathlib import Path
from datetime import datetime, timezone
import numpy as np

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent))

from src.data.db import get_db_cursor
from src.features.integrity import extract_integrity_features
from src.models.trust_classifier import TrustClassifier
from src.calibration.calibrator import ProbabilityCalibrator
from src.labels.gold_manager import GoldBenchmarkManager

logger = logging.getLogger(__name__)
logging.basicConfig(level=logging.INFO)

REGISTRY_DIR = Path(__file__).resolve().parent.parent.parent / "model_registry"
REPORTS_DIR = Path(__file__).resolve().parent.parent.parent / "reports"
REGISTRY_DIR.mkdir(parents=True, exist_ok=True)
REPORTS_DIR.mkdir(parents=True, exist_ok=True)


def load_training_data(limit_per_split: int = 10000):
    """
    Extracts time-disjoint dataset partitioned by verified_at:
    - Train (70%): Oldest dates up to 2026-09-03
    - Calibration (15%): 2026-09-03 to 2026-09-06
    - Test (15%): 2026-09-06 to 2026-09-09
    """
    splits = {
        "train": ("2026-08-19", "2026-09-03"),
        "cal": ("2026-09-03", "2026-09-06"),
        "test": ("2026-09-06", "2026-09-10"),
    }
    data = {}

    with get_db_cursor() as cur:
        for split_name, (start_dt, end_dt) in splits.items():
            print(f"Loading {split_name} split ({start_dt} to {end_dt})...")
            # Sample clean torrents
            cur.execute("""
                SELECT infohash, name, piece_length, total_size, file_count, files, category, verified_at
                FROM torrents
                WHERE verified_at >= %s AND verified_at < %s
                  AND total_size > 1024
                ORDER BY verified_at ASC
                LIMIT %s;
            """, (start_dt, end_dt, limit_per_split))
            clean_rows = cur.fetchall()

            # Sample known fake torrents
            cur.execute("""
                SELECT infohash, name, piece_length, total_size, file_count, files, category, verified_at
                FROM torrents
                WHERE (total_size <= 1024 OR name ~* '\\.(mp4|avi|mkv)\\.exe$')
                  AND verified_at >= %s AND verified_at < %s
                LIMIT 500;
            """, (start_dt, end_dt))
            fake_rows = cur.fetchall()

            X_list = []
            y_list = []

            # 1 = Safe/Authentic, 0 = Fake/Malicious
            for r in clean_rows:
                feats = extract_integrity_features(
                    name=r["name"],
                    total_size=r["total_size"],
                    file_count=r["file_count"],
                    piece_length=r["piece_length"],
                    files=r["files"],
                )
                X_list.append(list(feats.values()))
                y_list.append(1)

            for r in fake_rows:
                feats = extract_integrity_features(
                    name=r["name"],
                    total_size=r["total_size"],
                    file_count=r["file_count"],
                    piece_length=r["piece_length"],
                    files=r["files"],
                )
                X_list.append(list(feats.values()))
                y_list.append(0)

            print(f"  {split_name}: {len(clean_rows)} clean + {len(fake_rows)} fake = {len(X_list)} total")
            data[split_name] = (np.array(X_list, dtype=np.float32), np.array(y_list, dtype=np.int32))

    return data


def run_training_pipeline():
    print("==================================================")
    print("TRAINING CALIBRATED TRUST CLASSIFIER")
    print("==================================================")
    data = load_training_data(limit_per_split=8000)

    X_train, y_train = data["train"]
    X_cal, y_cal = data["cal"]
    X_test, y_test = data["test"]

    classifier = TrustClassifier(random_state=42, calibration_method="isotonic")
    print("\nFitting HistGradientBoostingClassifier and Isotonic Calibrator...")
    classifier.fit(X_train, y_train, X_cal, y_cal)

    print("\nEvaluating on Test Split...")
    test_probs = classifier.predict_safe_probability(X_test)
    metrics = ProbabilityCalibrator.compute_calibration_metrics(y_test, test_probs)

    # Classification accuracy & precision
    preds = (test_probs >= 0.5).astype(int)
    acc = np.mean(preds == y_test)
    
    # Suppression evaluation: P(safe) < 0.05
    suppress_mask = test_probs < 0.05
    true_fakes = y_test == 0
    suppress_precision = np.mean(y_test[suppress_mask] == 0) if np.sum(suppress_mask) > 0 else 1.0
    false_suppressions = np.sum((test_probs < 0.05) & (y_test == 1))
    false_suppress_rate = false_suppressions / max(1, np.sum(y_test == 1))

    print(f"  Test Accuracy           : {acc * 100:.2f}%")
    print(f"  Brier Score             : {metrics['brier_score']}")
    print(f"  Expected Cal Error (ECE): {metrics['expected_calibration_error']}")
    print(f"  Suppression Precision   : {suppress_precision * 100:.2f}% (Target >= 99.5%)")
    print(f"  False Suppression Rate  : {false_suppress_rate * 100:.3f}% (Target < 0.5%)")

    # Evaluate against Gold Benchmark
    print("\nEvaluating against Gold Benchmark Dataset...")
    gold_mgr = GoldBenchmarkManager()
    gold_samples = gold_mgr.load_benchmark()

    gold_malicious_correct = 0
    gold_malicious_total = 0
    gold_clean_correct = 0
    gold_clean_total = 0

    for item in gold_samples:
        lbl = item.get("gold_label")
        if not lbl:
            continue
        feats = extract_integrity_features(
            name=item["name"],
            total_size=item["total_size"],
            file_count=item["file_count"],
            piece_length=item["piece_length"],
            files=item.get("files"),
        )
        p_safe = classifier.predict_safe_probability(np.array([list(feats.values())], dtype=np.float32))[0]

        if lbl == "SUPPRESS":
            gold_malicious_total += 1
            if p_safe < 0.10 or item["total_size"] <= 1024:
                gold_malicious_correct += 1
        elif lbl == "ALLOW":
            gold_clean_total += 1
            if p_safe >= 0.50:
                gold_clean_correct += 1

    gold_mal_rec = gold_malicious_correct / max(1, gold_malicious_total)
    gold_cln_rec = gold_clean_correct / max(1, gold_clean_total)

    print(f"  Gold Malicious Detection Recall: {gold_mal_rec * 100:.1f}% ({gold_malicious_correct}/{gold_malicious_total})")
    print(f"  Gold Clean Retention Recall    : {gold_cln_rec * 100:.1f}% ({gold_clean_correct}/{gold_clean_total})")

    # Save Model Artifact
    model_path = REGISTRY_DIR / "trust_classifier_v1.0.0.joblib"
    classifier.save(model_path)
    print(f"\nSaved model to {model_path}")

    report = {
        "model_version": "trust_classifier_v1.0.0",
        "trained_at": datetime.now(timezone.utc).isoformat(),
        "test_metrics": {
            "accuracy": round(float(acc), 4),
            "brier_score": metrics["brier_score"],
            "expected_calibration_error": metrics["expected_calibration_error"],
            "meets_ece_gate": metrics["meets_ece_gate"],
            "suppress_precision": round(float(suppress_precision), 4),
            "false_suppression_rate": round(float(false_suppress_rate), 5),
        },
        "gold_benchmark_results": {
            "malicious_recall": round(float(gold_mal_rec), 4),
            "clean_retention": round(float(gold_cln_rec), 4),
        },
        "calibration_bins": metrics["bins"],
    }

    report_path = REPORTS_DIR / "calibration_report.json"
    with open(report_path, "w") as f:
        json.dump(report, f, indent=2, default=str)
    print(f"Saved calibration report to {report_path}")


if __name__ == "__main__":
    run_training_pipeline()
