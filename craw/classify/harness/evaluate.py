#!/usr/bin/env python3
"""Evaluate classifier predictions against gold labels."""

import argparse
import json
import sys
from collections import defaultdict

CATEGORIES = sorted(["Movies","Television","Games","Music","Applications","Anime","Documentaries","Other","Unwanted"])

def load_labels(path):
    """Load gold labels from label_map or train/eval labels JSON."""
    with open(path) as f:
        data = json.load(f)
    return {str(r["idx"]): r for r in data}

def load_predictions(path):
    with open(path) as f:
        data = json.load(f)
    return {str(r["idx"]): r for r in data}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--predictions", required=True, help="Path to predictions JSON")
    parser.add_argument("--labels", required=True, help="Path to gold labels JSON (eval only)")
    parser.add_argument("--subset", choices=["train", "eval"], default="eval")
    args = parser.parse_args()

    preds = load_predictions(args.predictions)
    labels = load_labels(args.labels)

    # Confusion matrix: gold_cat -> pred_cat -> count
    cm = defaultdict(lambda: defaultdict(int))
    keep_cm = defaultdict(int)  # (gold_keep, pred_keep) -> count
    n_malformed = 0
    n_total = 0
    n_correct = 0
    n_keep_correct = 0
    explicit_tp = 0
    explicit_fp = 0
    explicit_fn = 0

    for idx, gold in labels.items():
        n_total += 1
        pred = preds.get(idx)
        if pred is None:
            n_malformed += 1
            cm[gold["label_category"]]["MALFORMED"] += 1
            continue

        parsed = pred["parsed"]
        if parsed is None:
            n_malformed += 1
            cm[gold["label_category"]]["MALFORMED"] += 1
            continue

        pred_cat = parsed["category"]
        gold_cat = gold["label_category"]
        gold_keep = gold["label_keep"]
        pred_keep = parsed["keep"]

        cm[gold_cat][pred_cat] += 1
        keep_cm[(gold_keep, pred_keep)] += 1

        if pred_cat == gold_cat:
            n_correct += 1
        if pred_keep == gold_keep:
            n_keep_correct += 1

        # Explicit detection metrics
        gold_explicit = gold.get("regex_explicit", False) or gold_cat == "Unwanted"
        pred_explicit = parsed["explicit"]
        if pred_explicit and gold_explicit:
            explicit_tp += 1
        elif pred_explicit and not gold_explicit:
            explicit_fp += 1
        elif not pred_explicit and gold_explicit:
            explicit_fn += 1

    print(f"=== {args.subset.upper()} EVALUATION ===")
    print(f"Total: {n_total} | Malformed: {n_malformed} ({n_malformed/n_total*100:.1f}%)")
    print(f"Category accuracy: {n_correct}/{n_total - n_malformed} ({n_correct/(n_total - n_malformed)*100:.1f}%)")
    print(f"Keep accuracy: {n_keep_correct}/{n_total - n_malformed} ({n_keep_correct/(n_total - n_malformed)*100:.1f}%)")
    print()

    # Keep precision/recall for keep=false
    tn, fp = keep_cm[(True, True)], keep_cm[(True, False)]
    fn_keep, tp = keep_cm[(False, True)], keep_cm[(False, False)]
    print(f"Keep=false: TP={tp} FP={fp} FN={fn_keep} TN={tn}")
    if tp + fp > 0:
        prec = tp / (tp + fp)
        print(f"  Precision: {prec:.4f}")
    if tp + fn_keep > 0:
        rec = tp / (tp + fn_keep)
        print(f"  Recall: {rec:.4f}")
    if tp + fp > 0 and tp + fn_keep > 0:
        p = tp / (tp + fp)
        r = tp / (tp + fn_keep)
        print(f"  F1: {2*p*r/(p+r):.4f}")
    print()

    # Per-category P/R/F1
    print(f"{'Category':<20} {'P':>6} {'R':>6} {'F1':>6} {'Support':>8} {'Pred':>6}")
    print("-" * 60)
    for cat in CATEGORIES:
        support = sum(cm[cat].values())
        if support == 0:
            continue
        tp_cat = cm[cat][cat]
        pred_total = sum(cm[g][cat] for g in CATEGORIES + ["MALFORMED"])
        p = tp_cat / pred_total if pred_total > 0 else 0
        r = tp_cat / support if support > 0 else 0
        f1 = 2*p*r/(p+r) if p + r > 0 else 0
        print(f"{cat:<20} {p:>6.3f} {r:>6.3f} {f1:>6.3f} {support:>8} {pred_total:>6}")
    print()

    # Confusion matrix
    print("Confusion matrix (gold rows, pred cols):")
    header = f"{'Gold\\Pred':<20}" + "".join(f"{c[:6]:>7}" for c in CATEGORIES + ["MALF"])
    print(header)
    for gold_cat in CATEGORIES:
        row = f"{gold_cat:<20}"
        for pred_cat in CATEGORIES + ["MALFORMED"]:
            val = cm[gold_cat][pred_cat]
            row += f"{val:>7}"
        print(row)

if __name__ == "__main__":
    main()
