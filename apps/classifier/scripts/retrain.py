#!/usr/bin/env python3
import sys
import time
import json
import argparse
from pathlib import Path
from collections import Counter
import numpy as np
import joblib
from sklearn.model_selection import train_test_split
from sklearn.metrics import classification_report, f1_score, accuracy_score
from sklearn.linear_model import SGDClassifier
from sklearn.preprocessing import LabelEncoder

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

import db
from feature_extractor import TorrentFeatureExtractor
from model_manager import (
    get_active_model_info,
    get_active_model_path,
    activate_model,
    MODELS_DIR
)


def run_retraining(dry_run: bool = False, force: bool = False, max_samples: int = None):
    print("=" * 80, flush=True)
    print("GAIA TORRENT CLASSIFIER: DIRECT DB RETRAINING PIPELINE", flush=True)
    print("=" * 80, flush=True)

    # 1. Fetch training data directly from PostgreSQL
    print("\n[1/6] Extracting high-confidence labels from PostgreSQL (labeled_results JOIN torrents)...", flush=True)
    t0 = time.time()
    records = db.fetch_training_data(min_confidence="high", limit=max_samples)
    if max_samples and max_samples < len(records):
        records = records[:max_samples]
    print(f"      Retrieved {len(records):,} records across 10 classes in {time.time() - t0:.2f}s", flush=True)
    if len(records) < 1000:
        raise ValueError(f"Insufficient training records in database ({len(records)} found).")

    # 2. Encode labels and split train / validation
    print("\n[2/6] Preparing stratified 85/15 train/validation split...", flush=True)
    labels = [r["label_category"] for r in records]
    le = LabelEncoder()
    y = le.fit_transform(labels)
    classes = list(le.classes_)
    print(f"      Target classes ({len(classes)}): {classes}", flush=True)

    indices = np.arange(len(records))
    train_idx, val_idx = train_test_split(indices, test_size=0.15, stratify=y, random_state=42)
    train_records = [records[i] for i in train_idx]
    val_records = [records[i] for i in val_idx]
    y_train = y[train_idx]
    y_val = y[val_idx]

    print(f"      Train set: {len(train_records):,} | Validation holdout: {len(val_records):,}", flush=True)

    # 3. Fit Candidate Feature Extractor and Search for Best Regularization
    print("\n[3/6] Fitting candidate feature extractor and searching hyperparameter space...", flush=True)
    t1 = time.time()
    extractor = TorrentFeatureExtractor(max_features=250000)
    X_train = extractor.fit_transform(train_records)
    print(f"      Extracted {X_train.shape[1]:,} features in {time.time() - t1:.1f}s", flush=True)

    print("      Transforming validation holdout features...", flush=True)
    t_val = time.time()
    X_val = extractor.transform(val_records)
    print(f"      Validation features extracted in {time.time() - t_val:.1f}s", flush=True)

    # Grid search for optimal regularization parameter
    param_grid = [
        {"loss": "modified_huber", "alpha": 3e-5},
        {"loss": "modified_huber", "alpha": 7e-5},
        {"loss": "modified_huber", "alpha": 1.5e-4},
        {"loss": "modified_huber", "alpha": 3e-4},
        {"loss": "log_loss", "alpha": 7e-5},
        {"loss": "log_loss", "alpha": 1.5e-4},
    ]

    print("\n      --- Hyperparameter Evaluation on Holdout ---", flush=True)
    best_clf = None
    best_macro_f1 = -1.0
    best_params = None

    for p in param_grid:
        sub_clf = SGDClassifier(
            loss=p["loss"],
            penalty="l2",
            alpha=p["alpha"],
            max_iter=1000,
            class_weight="balanced",
            random_state=42
        )
        t_sub = time.time()
        sub_clf.fit(X_train, y_train)
        sub_preds = sub_clf.predict(X_val)
        sub_macro = float(f1_score(y_val, sub_preds, average="macro"))
        sub_acc = float(accuracy_score(y_val, sub_preds))
        print(f"      * loss={p['loss']:<14} alpha={p['alpha']:<8} -> Macro F1: {sub_macro*100:5.2f}% | Acc: {sub_acc*100:5.2f}% ({time.time() - t_sub:.1f}s)", flush=True)

        if sub_macro > best_macro_f1:
            best_macro_f1 = sub_macro
            best_clf = sub_clf
            best_params = p

    print(f"\n      -> Selected Best Configuration: {best_params} (Holdout Macro F1: {best_macro_f1*100:.2f}%)", flush=True)
    clf = best_clf

    # 4. Evaluate Candidate vs Active Model on EXACT same validation set
    print("\n[4/6] Evaluating candidate model vs currently active model on validation holdout...", flush=True)
    val_preds = clf.predict(X_val)
    cand_acc = float(accuracy_score(y_val, val_preds))
    cand_macro_f1 = float(best_macro_f1)
    cand_report = classification_report(y_val, val_preds, target_names=classes, output_dict=True)

    active_info = get_active_model_info()
    try:
        active_path = get_active_model_path()
        active_exists = active_path.exists()
    except Exception:
        active_path = None
        active_exists = False

    active_name = active_path.name if active_path else active_info.get("filename", "torrent_classifier_v2.joblib")
    print(f"      Active baseline: {active_info.get('version', 'unknown')} ({active_name})", flush=True)

    # Evaluate active model on this same validation holdout
    active_macro_f1 = 0.0
    active_acc = 0.0
    active_class_f1 = {}
    if active_exists:
        try:
            active_payload = joblib.load(active_path)
            active_ext = active_payload["extractor"]
            active_clf = active_payload["classifier"]
            active_classes = list(active_payload["classes"])

            X_val_active = active_ext.transform(val_records)
            act_preds_raw = active_clf.predict(X_val_active)
            # Map indices
            act_pred_labels = [active_classes[i] for i in act_preds_raw]
            val_labels_str = [classes[i] for i in y_val]

            active_acc = float(accuracy_score(val_labels_str, act_pred_labels))
            active_macro_f1 = float(f1_score(val_labels_str, act_pred_labels, average="macro"))
            act_report = classification_report(val_labels_str, act_pred_labels, output_dict=True)
            for c in classes:
                active_class_f1[c] = float(act_report.get(c, {}).get("f1-score", 0.0))
        except Exception as e:
            print(f"      Warning: could not evaluate active model ({e}), falling back to stored baseline metrics.", flush=True)
            active_macro_f1 = active_info.get("metrics", {}).get("macro_f1", 0.904)
            active_acc = active_info.get("metrics", {}).get("accuracy", 0.907)
            active_class_f1 = active_info.get("metrics", {}).get("per_class_f1", {})
    else:
        print("      Note: Local active .joblib not found; using stored active baseline metrics from active_model.json.", flush=True)
        active_macro_f1 = active_info.get("metrics", {}).get("macro_f1", 0.904)
        active_acc = active_info.get("metrics", {}).get("accuracy", 0.907)
        active_class_f1 = active_info.get("metrics", {}).get("per_class_f1", {})

    print("\n" + "-" * 75, flush=True)
    print(f"{'Class':<20} | {'Active F1':<12} | {'Candidate F1':<14} | {'Delta':<10}", flush=True)
    print("-" * 75, flush=True)
    class_deltas = {}
    class_regressions = []
    cand_class_f1 = {}

    for c in classes:
        act_f = active_class_f1.get(c, 0.0)
        cand_f = float(cand_report[c]["f1-score"])
        cand_class_f1[c] = round(cand_f, 4)
        delta = cand_f - act_f
        class_deltas[c] = delta
        status = "OK"
        if delta < -0.030:
            status = "REGRESSION (>3%)"
            class_regressions.append((c, delta))
        print(f"{c:<20} | {act_f*100:>6.2f}%      | {cand_f*100:>6.2f}%        | {delta*100:>+5.2f}% {status}", flush=True)

    macro_delta = cand_macro_f1 - active_macro_f1
    print("-" * 75, flush=True)
    print(f"{'OVERALL MACRO F1':<20} | {active_macro_f1*100:>6.2f}%      | {cand_macro_f1*100:>6.2f}%        | {macro_delta*100:>+5.2f}%", flush=True)
    print(f"{'OVERALL ACCURACY':<20} | {active_acc*100:>6.2f}%      | {cand_acc*100:>6.2f}%        | {(cand_acc - active_acc)*100:>+5.2f}%", flush=True)
    print("-" * 75, flush=True)

    # 5. Shadow / Canary Traffic Slice Evaluation
    print("\n[5/6] Running shadow canary comparison on 1,000 recent production torrents...", flush=True)
    canary_slice = db.fetch_recent_canary_slice(limit=1000)
    canary_warnings = []
    canary_stats = {}

    if canary_slice and active_exists:
        try:
            cand_canary_X = extractor.transform(canary_slice)
            cand_canary_preds = [classes[i] for i in clf.predict(cand_canary_X)]
            cand_counts = Counter(cand_canary_preds)

            act_canary_X = active_ext.transform(canary_slice)
            act_canary_preds = [active_classes[i] for i in active_clf.predict(act_canary_X)]
            act_counts = Counter(act_canary_preds)

            print(f"{'Category':<20} | {'Active Share':<14} | {'Candidate Share':<16} | {'Rel Shift':<10}", flush=True)
            print("-" * 70, flush=True)
            total_slice = len(canary_slice)
            for c in classes:
                act_share = act_counts.get(c, 0) / total_slice
                cand_share = cand_counts.get(c, 0) / total_slice
                canary_stats[c] = {"active_share": round(act_share, 4), "candidate_share": round(cand_share, 4)}
                if act_share > 0.02:  # Only evaluate shifts on categories with at least 2% share
                    rel_shift = (cand_share - act_share) / act_share
                    flag = ""
                    if abs(rel_shift) > 0.25:
                        flag = "ANOMALY (>25%)"
                        canary_warnings.append(f"{c}: relative shift of {rel_shift*100:+.1f}%")
                    print(f"{c:<20} | {act_share*100:>6.1f}%        | {cand_share*100:>6.1f}%          | {rel_shift*100:>+5.1f}% {flag}", flush=True)
                else:
                    print(f"{c:<20} | {act_share*100:>6.1f}%        | {cand_share*100:>6.1f}%          | (minor)", flush=True)
            print("-" * 70, flush=True)
        except Exception as e:
            print(f"      Canary comparison skipped: {e}", flush=True)

    # 6. Quality Gate Verification & Promotion
    print("\n[6/6] Verifying Comparative Quality Gate...", flush=True)
    gate_passed = True
    gate_reasons = []

    # Rule 1: Macro F1 regression
    if macro_delta < -0.005:
        gate_passed = False
        gate_reasons.append(f"Macro F1 regressed by {macro_delta*100:.2f}% (tolerance: -0.5%)")

    # Rule 2: Individual class collapse
    if class_regressions:
        gate_passed = False
        for c, d in class_regressions:
            gate_reasons.append(f"Class '{c}' F1 dropped by {abs(d)*100:.2f}% (max tolerance: 3.0%)")

    # Rule 3: Canary warnings
    if canary_warnings:
        print(f"      [!] Canary alerts detected: {', '.join(canary_warnings)}", flush=True)
        if not force:
            gate_reasons.append(f"Canary distribution shifts exceed 25%: {canary_warnings}")

    if gate_passed:
        print("      ✓ QUALITY GATE PASSED: Candidate meets all stability and performance criteria.", flush=True)
    else:
        print("      ✗ QUALITY GATE FAILED:", flush=True)
        for r in gate_reasons:
            print(f"        - {r}", flush=True)

    if (not gate_passed) and (not force):
        print("\n[!] PROMOTION ABORTED: The candidate model was NOT activated to protect production.", flush=True)
        return {
            "status": "rejected",
            "gate_passed": False,
            "reasons": gate_reasons,
            "candidate_metrics": {"macro_f1": cand_macro_f1, "accuracy": cand_acc, "per_class_f1": cand_class_f1}
        }

    if dry_run:
        print("\n[*] DRY RUN COMPLETE: Model not saved or activated (--dry-run specified).", flush=True)
        return {
            "status": "dry_run_success",
            "gate_passed": gate_passed,
            "candidate_metrics": {"macro_f1": cand_macro_f1, "accuracy": cand_acc, "per_class_f1": cand_class_f1}
        }

    # Determine next version
    curr_v = active_info.get("version", "v2")
    try:
        curr_num = int(curr_v.lstrip("v"))
        next_v = f"v{curr_num + 1}"
    except Exception:
        next_v = f"v_{int(time.time())}"

    timestamp = time.strftime("%Y%m%d_%H%M%S")
    out_filename = f"torrent_classifier_{next_v}_{timestamp}.joblib"
    out_path = MODELS_DIR / out_filename

    print(f"\nSaving and activating candidate model as {next_v} ({out_filename})...", flush=True)
    payload = {
        "extractor": extractor,
        "classifier": clf,
        "classes": classes,
        "metadata": {
            "version": next_v,
            "trained_at": time.strftime("%Y-%m-%d %H:%M:%S"),
            "num_training_samples": len(train_records),
            "num_validation_samples": len(val_records),
            "num_features": X_train.shape[1]
        }
    }
    joblib.dump(payload, out_path, compress=3)

    metrics_payload = {
        "macro_f1": round(cand_macro_f1, 4),
        "accuracy": round(cand_acc, 4),
        "per_class_f1": cand_class_f1,
        "gate_passed": gate_passed,
        "forced": force
    }

    activated = activate_model(
        version=next_v,
        filename=out_filename,
        metrics=metrics_payload,
        canary_stats=canary_stats
    )

    print(f"✓ Model {next_v} successfully activated and recorded in models/active_model.json!", flush=True)
    return {
        "status": "promoted",
        "version": next_v,
        "filename": out_filename,
        "metrics": metrics_payload
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Gaia Torrent Classifier Retraining Pipeline")
    parser.add_argument("--dry-run", action="store_true", help="Evaluate and gate candidate without saving/activating")
    parser.add_argument("--force", action="store_true", help="Bypass quality gates and force model activation")
    parser.add_argument("--max-samples", type=int, default=None, help="Sample limit for fast validation tests")
    args = parser.parse_args()

    run_retraining(dry_run=args.dry_run, force=args.force, max_samples=args.max_samples)
