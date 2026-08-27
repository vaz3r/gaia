"""
Threshold tuner using a proper held-out validation set.
Usage: python tune_threshold_val.py
Outputs the best threshold per-class and globally, then evaluates
that threshold on the final 1,000-sample test set.
"""
import json
import numpy as np
from collections import Counter


def load_gold(path, hash_key="infohash"):
    gold = {}
    with open(path) as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                h = r.get(hash_key, r.get("id", ""))
                label = r.get("label_category", r.get("TRUE_LABEL", ""))
                if h and label:
                    gold[h] = label
    return gold


def load_preds(path):
    preds = {}
    with open(path) as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                h = r.get("infohash", r.get("id", ""))
                preds[h] = r
    return preds


def compute_macro_f1(gold, preds, thresh=0.0, cats=None):
    y_true, y_pred = [], []
    for h, g in gold.items():
        if h not in preds:
            continue
        p = preds[h]
        cat = p.get("category", "Other")
        conf = float(p.get("confidence", 1.0))
        if cat != "Other" and conf < thresh:
            cat = "Other"
        y_true.append(g)
        y_pred.append(cat)

    cats = cats or sorted(set(y_true))
    f1s = {}
    for cat in cats:
        tp = sum(1 for t, p in zip(y_true, y_pred) if t == cat and p == cat)
        fp = sum(1 for t, p in zip(y_true, y_pred) if t != cat and p == cat)
        fn = sum(1 for t, p in zip(y_true, y_pred) if t == cat and p != cat)
        pr = tp / (tp + fp) if (tp + fp) else 0
        rc = tp / (tp + fn) if (tp + fn) else 0
        f1 = 2 * pr * rc / (pr + rc) if (pr + rc) else 0
        f1s[cat] = {"p": pr, "r": rc, "f1": f1, "tp": tp, "fp": fp, "fn": fn,
                    "support": y_true.count(cat)}
    macro = np.mean([v["f1"] for v in f1s.values()])
    acc = sum(t == p for t, p in zip(y_true, y_pred)) / len(y_true) if y_true else 0
    return macro, f1s, acc


def print_report(title, macro, f1s, acc):
    print(f"\n=== {title} ===")
    print(f"Accuracy: {acc:.3f}  |  Macro F1: {macro:.4f}")
    print(f"{'Category':<18} {'P':>6} {'R':>6} {'F1':>7} {'Support':>8}")
    print("-" * 50)
    for cat, v in sorted(f1s.items()):
        print(f"{cat:<18} {v['p']:>6.3f} {v['r']:>6.3f} {v['f1']:>7.3f} {v['support']:>8}")
    print("-" * 50)
    print(f"{'Macro avg':<18} {'':>6} {'':>6} {macro:>7.4f}")


# ── Load validation set ───────────────────────────────────────────────────────
val_gold = load_gold("data/val_v10_20pct.jsonl")
val_preds = load_preds("data/predictions_val_v10.jsonl")
cats = sorted(set(val_gold.values()))

print(f"Validation set: {len(val_gold)} items, {len(val_preds)} predictions")
print(f"Categories: {cats}")

# ── Baseline on val set ───────────────────────────────────────────────────────
baseline_f1, baseline_f1s, baseline_acc = compute_macro_f1(val_gold, val_preds, thresh=0.0)
print_report("Baseline (no threshold, val set)", baseline_f1, baseline_f1s, baseline_acc)

# ── Sweep global threshold on val set ────────────────────────────────────────
print("\n\nGlobal threshold sweep (on VALIDATION set):")
print(f"{'Threshold':>10} {'Macro F1':>10} {'Movies P':>10} {'Movies R':>10} {'Other P':>9}")
best_thresh, best_f1 = 0.0, baseline_f1
results = []
for t in np.arange(0.30, 0.70, 0.02):
    mf1, f1s, acc = compute_macro_f1(val_gold, val_preds, thresh=t)
    mp = f1s.get("Movies", {}).get("p", 0)
    mr = f1s.get("Movies", {}).get("r", 0)
    op = f1s.get("Other", {}).get("p", 0)
    results.append((t, mf1, mp, mr, op))
    marker = " ◄" if mf1 > best_f1 else ""
    print(f"{t:>10.2f} {mf1:>10.4f} {mp:>10.3f} {mr:>10.3f} {op:>9.3f}{marker}")
    if mf1 > best_f1:
        best_f1 = mf1
        best_thresh = t

print(f"\n✅ Best threshold on val set: {best_thresh:.2f}  (Macro F1: {best_f1:.4f})")

# ── Evaluate best threshold on FINAL TEST SET (honest evaluation) ─────────────
print("\n\n" + "=" * 60)
print("FINAL HONEST EVALUATION on 1,000-sample natural test set")
print("=" * 60)

test_gold = load_gold("data/manual_eval_set_1000.jsonl")
test_preds = load_preds("data/predictions_iter10_final.jsonl")

# Baseline (no threshold)
test_base_f1, test_base_f1s, test_base_acc = compute_macro_f1(test_gold, test_preds, thresh=0.0)
print_report("Test set: No threshold (baseline)", test_base_f1, test_base_f1s, test_base_acc)

# Tuned threshold
test_tuned_f1, test_tuned_f1s, test_tuned_acc = compute_macro_f1(test_gold, test_preds, thresh=best_thresh)
print_report(f"Test set: Threshold={best_thresh:.2f} (tuned on val)", test_tuned_f1, test_tuned_f1s, test_tuned_acc)

print(f"\n📊 Summary:")
print(f"  Threshold tuned on val set: {best_thresh:.2f}")
print(f"  Val Macro F1 (baseline → tuned): {baseline_f1:.4f} → {best_f1:.4f}")
print(f"  Test Macro F1 (baseline → tuned): {test_base_f1:.4f} → {test_tuned_f1:.4f}")
print(f"  Test Movies precision (baseline → tuned): {test_base_f1s.get('Movies',{}).get('p',0):.3f} → {test_tuned_f1s.get('Movies',{}).get('p',0):.3f}")
print(f"  Test Movies recall   (baseline → tuned): {test_base_f1s.get('Movies',{}).get('r',0):.3f} → {test_tuned_f1s.get('Movies',{}).get('r',0):.3f}")
