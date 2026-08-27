#!/usr/bin/env python3
"""
Merge true labels, new AL true labels, and pseudo-labels into a single training set.
Enforces deduplication against test/eval sets and among components.
"""
import json
from collections import Counter
from pathlib import Path

def merge_datasets():
    seen_hashes = set()
    
    # Exclude golden test set strictly
    test_hashes = set()
    with open("data/manual_eval_set_1000.jsonl") as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                test_hashes.add(r.get("infohash", r.get("id", "")).strip().lower())
                
    with open("data/manual_eval_set_200.jsonl") as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                test_hashes.add(r.get("infohash", r.get("id", "")).strip().lower())
                
    print(f"Loaded {len(test_hashes)} test set hashes to exclude.")
    
    # 1. Base True Labels (v10)
    base_true = []
    with open("data/training_combined_v10_true.jsonl") as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                h = r.get("infohash", r.get("id", "")).strip().lower()
                if h and h not in test_hashes and h not in seen_hashes:
                    seen_hashes.add(h)
                    r["sample_weight"] = 1.0
                    r["is_pseudo"] = False
                    base_true.append(r)
    print(f"1. Base True Labels: {len(base_true)}")
    
    # 2. New AL True Labels
    al_true = []
    with open("data/al_labeled_1129_true.jsonl") as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                h = r.get("infohash", r.get("id", "")).strip().lower()
                if h and h not in test_hashes and h not in seen_hashes:
                    seen_hashes.add(h)
                    r["sample_weight"] = 1.0
                    r["is_pseudo"] = False
                    al_true.append(r)
    print(f"2. New AL True Labels: {len(al_true)}")
    
    # 3. Pseudo-Labels (sample_weight=0.40)
    pseudo_labels = []
    with open("data/pseudo_labels_pool.jsonl") as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                h = r.get("infohash", r.get("id", "")).strip().lower()
                if h and h not in test_hashes and h not in seen_hashes:
                    seen_hashes.add(h)
                    r["sample_weight"] = 0.40
                    r["is_pseudo"] = True
                    pseudo_labels.append(r)
    print(f"3. Pseudo-Labels: {len(pseudo_labels)}")
    
    combined = base_true + al_true + pseudo_labels
    total_true = len(base_true) + len(al_true)
    total_pseudo = len(pseudo_labels)
    
    print("\n" + "="*50)
    print(f"COMBINED DATASET: {len(combined)} items")
    print(f"  True labels (weight 1.0):   {total_true}")
    print(f"  Pseudo-labels (weight 0.4): {total_pseudo}")
    print("="*50)
    
    print("\nTotal Class Distribution (Raw counts):")
    total_dist = Counter(r["label_category"] for r in combined)
    for cat, cnt in sorted(total_dist.items(), key=lambda x: -x[1]):
        print(f"  {cat:<18}: {cnt:5d} ({cnt/len(combined)*100:.1f}%)")
        
    print("\nTrue Labels Distribution:")
    true_dist = Counter(r["label_category"] for r in (base_true + al_true))
    for cat, cnt in sorted(true_dist.items(), key=lambda x: -x[1]):
        print(f"  {cat:<18}: {cnt:5d} ({cnt/total_true*100:.1f}%)")

    out_path = "data/training_semi_supervised_v1.jsonl"
    with open(out_path, "w", encoding="utf-8") as f:
        for r in combined:
            f.write(json.dumps(r) + "\n")
            
    print(f"\nSaved combined dataset to {out_path}")

if __name__ == "__main__":
    merge_datasets()
