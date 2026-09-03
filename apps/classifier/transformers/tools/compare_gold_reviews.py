#!/usr/bin/env python3
"""
Dual-Review Gold Annotation Comparator.
Computes inter-annotator agreement, Cohen's Kappa, per-class overlap,
and builds the Adjudication Queue without automatic resolution.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter, defaultdict
from pathlib import Path

FROZEN_CLASSES = sorted([
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
])


def compute_cohens_kappa(labels_a: list[str], labels_b: list[str], classes: list[str]) -> float:
    """Compute Cohen's Kappa coefficient between two raters."""
    assert len(labels_a) == len(labels_b)
    n = len(labels_a)
    if n == 0:
        return 0.0

    # Observed agreement
    p_o = sum(1 for a, b in zip(labels_a, labels_b) if a == b) / n

    # Expected agreement by chance
    counts_a = Counter(labels_a)
    counts_b = Counter(labels_b)

    p_e = sum((counts_a[c] / n) * (counts_b[c] / n) for c in classes)

    if p_e >= 1.0:
        return 1.0
    return (p_o - p_e) / (1.0 - p_e)


def compare_gold_reviews(
    review_a_path: str,
    review_b_path: str,
    manifest_path: str = "apps/classifier/data/gold_pilot/gold_pilot_manifest.json",
    blind_path: str = "apps/classifier/data/gold_pilot/gold_pilot_blind.jsonl",
    out_queue_path: str = "apps/classifier/data/gold_pilot/gold_pilot_adjudication_queue.jsonl",
) -> dict:
    """Compare two independent review sets and prepare adjudication queue."""
    print("=================================================================")
    print("DUAL-REVIEW GOLD ANNOTATION COMPARATOR")
    print("=================================================================")
    print(f"Reviewer A File: {review_a_path}")
    print(f"Reviewer B File: {review_b_path}")

    # Load annotations
    def load_annotations(path):
        rows = {}
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                if not line.strip():
                    continue
                r = json.loads(line)
                rows[r["pilot_id"]] = r
        return rows

    reviews_a = load_annotations(review_a_path)
    reviews_b = load_annotations(review_b_path)

    # Load blind metadata for adjudication queue context
    blind_info = {}
    if os.path.exists(blind_path):
        with open(blind_path, "r", encoding="utf-8") as f:
            for line in f:
                if not line.strip():
                    continue
                r = json.loads(line)
                blind_info[r["pilot_id"]] = r

    common_pids = sorted(list(set(reviews_a.keys()) & set(reviews_b.keys())))
    if len(common_pids) == 0:
        print("❌ Error: No common pilot IDs found between files.")
        return {}

    labels_a = [reviews_a[pid]["label_category"] for pid in common_pids]
    labels_b = [reviews_b[pid]["label_category"] for pid in common_pids]

    total = len(common_pids)
    exact_matches = sum(1 for a, b in zip(labels_a, labels_b) if a == b)
    raw_agreement = exact_matches / total
    kappa = compute_cohens_kappa(labels_a, labels_b, FROZEN_CLASSES)

    # Confusion matrix
    conf_matrix = defaultdict(lambda: defaultdict(int))
    for a, b in zip(labels_a, labels_b):
        conf_matrix[a][b] += 1

    # Per-class metrics
    per_class_stats = {}
    for c in FROZEN_CLASSES:
        a_count = sum(1 for x in labels_a if x == c)
        b_count = sum(1 for x in labels_b if x == c)
        agree_count = sum(1 for a, b in zip(labels_a, labels_b) if a == c and b == c)
        per_class_stats[c] = {
            "reviewer_a_count": a_count,
            "reviewer_b_count": b_count,
            "both_agreed": agree_count,
            "jaccard_agreement": agree_count / max(a_count + b_count - agree_count, 1),
        }

    # Adjudication Queue Preparation
    adjudication_items = []
    for pid in common_pids:
        ra = reviews_a[pid]
        rb = reviews_b[pid]
        disagreement = ra["label_category"] != rb["label_category"]
        low_confidence = ra.get("reviewer_confidence") == "low" or rb.get("reviewer_confidence") == "low"
        ambiguous = bool(ra.get("ambiguous")) or bool(rb.get("ambiguous"))
        adjudication_flag = bool(ra.get("adjudication_required")) or bool(rb.get("adjudication_required"))

        needs_adjudication = disagreement or low_confidence or ambiguous or adjudication_flag

        if needs_adjudication:
            b_meta = blind_info.get(pid, {})
            adjudication_items.append({
                "pilot_id": pid,
                "needs_adjudication": True,
                "disagreement": disagreement,
                "low_confidence": low_confidence,
                "ambiguous": ambiguous,
                "reviewer_a": {
                    "reviewer_id": ra.get("reviewer_id"),
                    "label": ra.get("label_category"),
                    "confidence": ra.get("reviewer_confidence"),
                    "alternate": ra.get("alternate_category"),
                    "reason": ra.get("reason"),
                },
                "reviewer_b": {
                    "reviewer_id": rb.get("reviewer_id"),
                    "label": rb.get("label_category"),
                    "confidence": rb.get("reviewer_confidence"),
                    "alternate": rb.get("alternate_category"),
                    "reason": rb.get("reason"),
                },
                "adjudicated_gold_label": None,
                "adjudicator_reason": None,
                "adjudication_timestamp": None,
                "adjudicator_id": None,
            })

    # Save adjudication queue
    out_q = Path(out_queue_path)
    out_q.parent.mkdir(parents=True, exist_ok=True)
    with open(out_q, "w", encoding="utf-8") as f:
        for item in adjudication_items:
            f.write(json.dumps(item) + "\n")

    print("\n-----------------------------------------------------------------")
    print(f"Total Evaluated Records:     {total}")
    print(f"Exact Agreement:             {exact_matches} / {total} ({raw_agreement * 100:.2f}%)")
    print(f"Cohen's Kappa (κ):           {kappa:.4f}")
    print(f"Disagreements:               {total - exact_matches}")
    print(f"Items Sent to Adjudication:  {len(adjudication_items)} ({len(adjudication_items) / total * 100:.1f}%)")
    print(f"Adjudication Queue File:     {out_q}")
    print("-----------------------------------------------------------------")

    print("\nPer-Class Jaccard Agreement:")
    for c, stats in per_class_stats.items():
        print(f"  {c:<15}: Agreed {stats['both_agreed']:>2} | RevA: {stats['reviewer_a_count']:>2} | RevB: {stats['reviewer_b_count']:>2} | Jaccard: {stats['jaccard_agreement'] * 100:.1f}%")

    print("\nReviewer Confusion Matrix (Rows: Reviewer A, Columns: Reviewer B):")
    hdr = f"{'':<14}" + "".join(f"{c[:5]:>7}" for c in FROZEN_CLASSES)
    print(hdr)
    for ca in FROZEN_CLASSES:
        row_str = f"{ca:<14}"
        for cb in FROZEN_CLASSES:
            val = conf_matrix[ca][cb]
            row_str += f"{val:>7}"
        print(row_str)

    return {
        "total": total,
        "exact_agreement": raw_agreement,
        "cohens_kappa": kappa,
        "adjudication_count": len(adjudication_items),
        "per_class_stats": per_class_stats,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Compare dual-review annotations.")
    parser.add_argument("review_a", help="Path to Reviewer A JSONL")
    parser.add_argument("review_b", help="Path to Reviewer B JSONL")
    parser.add_argument("--out_queue", default="apps/classifier/data/gold_pilot/gold_pilot_adjudication_queue.jsonl", help="Path to output adjudication queue")
    args = parser.parse_args()
    compare_gold_reviews(args.review_a, args.review_b, out_queue_path=args.out_queue)
