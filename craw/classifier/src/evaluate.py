#!/usr/bin/env python3
"""Evaluation harness for classifier predictions."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict

from src.core.types import ALLOWED_CATEGORIES


def load_jsonl(path: str) -> dict:
    data = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            key = row.get("infohash", row.get("idx", ""))
            data[str(key)] = row
    return data


def main():
    parser = argparse.ArgumentParser(description="Evaluate classifier predictions")
    parser.add_argument("--predictions", required=True, help="Predictions JSONL")
    parser.add_argument("--labels", required=True, help="Gold labels JSONL")
    parser.add_argument("--report", default=None, help="Output report file")
    args = parser.parse_args()

    preds = load_jsonl(args.predictions)
    labels = load_jsonl(args.labels)

    categories = sorted(ALLOWED_CATEGORIES)
    cm = defaultdict(lambda: defaultdict(int))
    n_total = 0
    n_correct = 0
    n_parse_fail = 0

    for key, gold in labels.items():
        n_total += 1
        pred = preds.get(key)

        gold_cat = gold.get("true_category", gold.get("label_category", ""))

        if pred is None:
            n_parse_fail += 1
            cm[gold_cat]["MALFORMED"] += 1
            continue

        parse_status = pred.get("parse_status", "success")
        pred_cat = pred.get("category", "Other")

        if parse_status != "success":
            n_parse_fail += 1
            cm[gold_cat]["MALFORMED"] += 1
            continue

        cm[gold_cat][pred_cat] += 1
        if pred_cat == gold_cat:
            n_correct += 1

    valid_total = n_total - n_parse_fail
    accuracy = n_correct / valid_total if valid_total > 0 else 0.0

    lines = []
    lines.append("=== Classification Report ===")
    lines.append(f"Total:       {n_total}")
    lines.append(f"Evaluated:   {valid_total}")
    lines.append(f"Parse fail:  {n_parse_fail} ({n_parse_fail/n_total*100:.1f}%)")
    lines.append(f"Accuracy:    {n_correct}/{valid_total} ({accuracy*100:.1f}%)")
    lines.append("")

    header = f"{'Category':<20} {'P':>6} {'R':>6} {'F1':>6} {'Support':>8}"
    lines.append(header)
    lines.append("-" * 52)

    macro_f1_sum = 0.0
    macro_p_sum = 0.0
    macro_r_sum = 0.0
    n_cats = 0

    for cat in categories:
        support = sum(cm[cat].values())
        if support == 0:
            continue
        tp = cm[cat][cat]
        pred_total = sum(cm[g][cat] for g in categories + ["MALFORMED"])
        p = tp / pred_total if pred_total > 0 else 0.0
        r = tp / support if support > 0 else 0.0
        f1 = 2 * p * r / (p + r) if p + r > 0 else 0.0
        lines.append(f"{cat:<20} {p:>6.3f} {r:>6.3f} {f1:>6.3f} {support:>8}")
        macro_p_sum += p
        macro_r_sum += r
        macro_f1_sum += f1
        n_cats += 1

    if n_cats > 0:
        lines.append("-" * 52)
        mp = macro_p_sum / n_cats
        mr = macro_r_sum / n_cats
        mf1 = macro_f1_sum / n_cats
        lines.append(f"{'Macro avg':<20} {mp:>6.3f} {mr:>6.3f} {mf1:>6.3f}")

    lines.append("")
    lines.append("Confusion matrix (gold rows, pred cols):")

    col_headers = "".join(f"{c[:8]:>9}" for c in categories + ["MALFORMED"])
    lines.append(f"{'Gold/Pred':<20}{col_headers}")

    for gold_cat in categories:
        row = f"{gold_cat:<20}"
        for pred_cat in categories + ["MALFORMED"]:
            val = cm[gold_cat][pred_cat]
            row += f"{val:>9}"
        lines.append(row)

    report = "\n".join(lines)
    print(report)

    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            f.write(report + "\n")
        print(f"\nReport written to {args.report}")


if __name__ == "__main__":
    main()
