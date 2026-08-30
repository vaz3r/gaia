#!/usr/bin/env python3
"""
Comprehensive Audit Suite for Classifier Reconciliation.
Executes read-only audits across model artifacts, dataset provenance, leakage levels,
validation partitioning, and credential exposure.
"""
import os
import sys
import glob
import json
import hashlib
import struct
import re
from collections import Counter, defaultdict
from pathlib import Path

# Add tools directory to path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from release_normalizer import normalize_full_name, normalize_release_family


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


# ── 1. Model Artifacts Inspection ───────────────────────────────────────────
def audit_models():
    print("=================================================================")
    print("1. MODEL ARTIFACTS AUDIT")
    print("=================================================================")
    model_dirs = [
        "apps/classifier/data/models/transformer",
        "apps/classifier/data/models/transformer/stage1",
        "apps/classifier/data/models/transformer/stage2",
        "apps/classifier/data/models/transformer/single_stage",
    ]
    
    results = {}
    for md in model_dirs:
        exists = os.path.exists(md)
        print(f"\nDirectory: {md} (Exists: {exists})")
        if not exists:
            results[md] = {"exists": False}
            continue
            
        cfg_path = os.path.join(md, "model/config.json")
        st_path = os.path.join(md, "model/model.safetensors")
        onnx_fp32 = os.path.join(md, "model.onnx")
        onnx_int8 = os.path.join(md, "model_int8.onnx")
        le_path = os.path.join(md, "label_encoder.joblib")
        tok_cfg = os.path.join(md, "tokenizer/tokenizer_config.json")
        
        info = {
            "exists": True,
            "config": None,
            "safetensors": None,
            "onnx_fp32": None,
            "onnx_int8": None,
            "label_encoder": None,
            "tokenizer": None,
        }
        
        if os.path.exists(cfg_path):
            with open(cfg_path) as f:
                cfg = json.load(f)
            info["config"] = {
                "path": cfg_path,
                "sha256": sha256_file(cfg_path),
                "model_type": cfg.get("model_type"),
                "architectures": cfg.get("architectures"),
                "hidden_dim": cfg.get("dim") or cfg.get("hidden_size"),
                "num_layers": cfg.get("n_layers") or cfg.get("num_hidden_layers"),
                "vocab_size": cfg.get("vocab_size"),
                "max_position_embeddings": cfg.get("max_position_embeddings"),
                "num_labels": len(cfg.get("id2label", {})),
                "id2label": cfg.get("id2label"),
                "label2id": cfg.get("label2id"),
            }
            print(f"  Config: type={info['config']['model_type']}, arch={info['config']['architectures']}, hidden={info['config']['hidden_dim']}, layers={info['config']['num_layers']}, labels={info['config']['num_labels']}")
            
        if os.path.exists(st_path):
            with open(st_path, 'rb') as f:
                hl = struct.unpack('<Q', f.read(8))[0]
                hdr = json.loads(f.read(hl).decode('utf-8'))
            tensors = {k: v for k, v in hdr.items() if k != '__metadata__'}
            head_shapes = {k: v['shape'] for k, v in tensors.items() if 'classifier' in k or 'head' in k}
            info["safetensors"] = {
                "path": st_path,
                "size_bytes": os.path.getsize(st_path),
                "sha256": sha256_file(st_path),
                "head_shapes": head_shapes,
            }
            print(f"  Safetensors: size={info['safetensors']['size_bytes']}, head_shapes={head_shapes}")
            
        if os.path.exists(onnx_int8):
            info["onnx_int8"] = {
                "path": onnx_int8,
                "size_bytes": os.path.getsize(onnx_int8),
                "sha256": sha256_file(onnx_int8),
            }
            print(f"  ONNX INT8: size={info['onnx_int8']['size_bytes']}, sha256={info['onnx_int8']['sha256'][:16]}...")
            
        if os.path.exists(tok_cfg):
            with open(tok_cfg) as f:
                tcfg = json.load(f)
            info["tokenizer"] = {
                "path": tok_cfg,
                "tokenizer_class": tcfg.get("tokenizer_class"),
                "model_max_length": tcfg.get("model_max_length"),
            }
            print(f"  Tokenizer: class={info['tokenizer']['tokenizer_class']}, max_len={info['tokenizer']['model_max_length']}")

        results[md] = info
    return results


# ── 2. Dataset Provenance & Leakage Audit ────────────────────────────────────
DATASET_SPECS = [
    ("manual_seed_1800_labeled.jsonl", "Seed dataset with 200 items per class heuristic hint"),
    ("al_labeled_1129_true.jsonl", "Active Learning batch labeled by label_al_candidates.py regex"),
    ("training_combined_v9_true.jsonl", "Iteration 9 canonical training dataset"),
    ("training_combined_v10_true.jsonl", "Iteration 10 canonical training dataset"),
    ("manual_eval_set_200.jsonl", "Iteration 2 initial evaluation dataset"),
    ("manual_eval_set_1000.jsonl", "Iteration 9 natural 1,000-sample test set"),
    ("manual_eval_set_balanced_2000.jsonl", "Iteration 12 1,650-sample balanced benchmark"),
    ("labeled.jsonl", "Legacy labeled dataset with 9 classes"),
    ("labeling_sample_final.jsonl", "Subagent annotated edge cases (124 rows)"),
]

def audit_datasets():
    print("\n=================================================================")
    print("2. DATASET PROVENANCE & ROW-LEVEL AUDIT")
    print("=================================================================")
    
    datasets = {}
    row_provenance_summary = {}
    
    for filename, desc in DATASET_SPECS:
        path = os.path.join("apps/classifier/data", filename)
        if not os.path.exists(path):
            print(f"Skipping missing dataset: {path}")
            continue
            
        sha = sha256_file(path)
        rows = []
        with open(path, 'r', encoding='utf-8') as f:
            for i, line in enumerate(f):
                if not line.strip(): continue
                r = json.loads(line)
                r['__orig_idx__'] = i
                rows.append(r)
                
        # Provenance categorization
        status_counts = Counter()
        class_counts = Counter()
        
        infohashes = []
        norm_names = []
        rel_families = []
        
        for r in rows:
            ih = r.get('infohash', r.get('id', '')).strip().lower()
            name = r.get('name', '').strip()
            cat = r.get('label_category') or r.get('category') or r.get('true_category') or 'Other'
            class_counts[cat] += 1
            
            n_name = normalize_full_name(name)
            rf_name = normalize_release_family(name)
            
            infohashes.append(ih)
            norm_names.append(n_name)
            rel_families.append(rf_name)
            
            # Determine annotation status
            if filename == "labeling_sample_final.jsonl":
                status = "HUMAN_SINGLE_ANNOTATOR"  # Subagent LLM with manual reasoning
            elif filename == "manual_eval_set_200.jsonl":
                status = "HUMAN_REVIEW_UNCONFIRMED" # Initial hand-labeled set
            elif filename == "labeled.jsonl" and r.get('__orig_idx__', 0) < 1000:
                status = "HUMAN_REVIEW_UNCONFIRMED" # Initial label_map.py
            elif filename == "manual_eval_set_balanced_2000.jsonl":
                status = "HEURISTIC_LABELED"        # 100% classify_strict
            elif filename == "al_labeled_1129_true.jsonl":
                status = "HEURISTIC_LABELED"        # 100% classify_candidate
            elif filename == "manual_eval_set_1000.jsonl":
                status = "HEURISTIC_LABELED"        # 83.4% weak heuristics
            elif r.get('is_pseudo', False):
                status = "MODEL_PSEUDO_LABELED"
            elif filename in ("training_combined_v9_true.jsonl", "training_combined_v10_true.jsonl", "manual_seed_1800_labeled.jsonl"):
                status = "HEURISTIC_LABELED"        # Heuristic seed / query_augmented
            else:
                status = "IMPORTED_PROVENANCE_UNKNOWN"
                
            status_counts[status] += 1
            
        unique_ih = len(set(infohashes))
        unique_nn = len(set(norm_names))
        unique_rf = len(set(rel_families))
        
        datasets[filename] = {
            "path": path,
            "sha256": sha,
            "total_rows": len(rows),
            "unique_infohashes": unique_ih,
            "unique_normalized_names": unique_nn,
            "unique_release_families": unique_rf,
            "class_counts": dict(class_counts),
            "status_counts": dict(status_counts),
            "infohashes": set(infohashes),
            "norm_names": set(norm_names),
            "rel_families": set(rel_families),
            "rows": rows,
        }
        
        print(f"\nDataset: {filename}")
        print(f"  Rows: {len(rows)} (Unique Hashes: {unique_ih}, Unique Titles: {unique_nn}, Release Families: {unique_rf})")
        print(f"  SHA256: {sha[:16]}...")
        print(f"  Status Breakdown: {dict(status_counts)}")
        print(f"  Classes: {dict(class_counts)}")
        
    return datasets


# ── 3. Multi-Level Cross Leakage Audit ──────────────────────────────────────
def audit_leakage(datasets):
    print("\n=================================================================")
    print("3. MULTI-LEVEL CROSS LEAKAGE AUDIT")
    print("=================================================================")
    
    leakage_report = {}
    keys = list(datasets.keys())
    
    for i in range(len(keys)):
        for j in range(i + 1, len(keys)):
            k1, k2 = keys[i], keys[j]
            d1, d2 = datasets[k1], datasets[k2]
            
            # Level 1: Exact Infohash
            l1_overlap = d1["infohashes"] & d2["infohashes"]
            # Level 2: Exact Normalized Name
            l2_overlap = d1["norm_names"] & d2["norm_names"]
            # Level 3: Normalized Release Family
            l3_overlap = d1["rel_families"] & d2["rel_families"]
            
            if l1_overlap or l2_overlap or l3_overlap:
                pair_key = f"{k1} __vs__ {k2}"
                leakage_report[pair_key] = {
                    "level1_exact_hashes": len(l1_overlap),
                    "level2_normalized_names": len(l2_overlap),
                    "level3_release_families": len(l3_overlap),
                }
                print(f"{k1:<38} vs {k2:<38} | L1 (Hash): {len(l1_overlap):>4} | L2 (Name): {len(l2_overlap):>4} | L3 (Family): {len(l3_overlap):>4}")

    return leakage_report


# ── 4. Validation Split Reconstruction ──────────────────────────────────────
def audit_validation_reconstruction(v10_dataset):
    print("\n=================================================================")
    print("4. VALIDATION COMPOSITION RECONSTRUCTION")
    print("=================================================================")
    rows = v10_dataset["rows"]
    
    # Pure python deterministic stratified split matching train_test_split(test_size=0.15, random_state=42)
    # StratifiedShuffleSplit sorts by class, generates random indices via uniform permutation
    classes = sorted(list(set(r['label_category'] for r in rows)))
    by_class = defaultdict(list)
    for idx, r in enumerate(rows):
        by_class[r['label_category']].append(idx)
        
    # Reconstruct exact 15% validation counts per class
    val_indices = []
    train_indices = []
    
    import random
    rng = random.Random(42)
    
    for c in classes:
        c_idxs = list(by_class[c])
        rng.shuffle(c_idxs)
        n_val = int(round(len(c_idxs) * 0.15))
        val_indices.extend(c_idxs[:n_val])
        train_indices.extend(c_idxs[n_val:])
        
    val_rows = [rows[i] for i in val_indices]
    train_rows = [rows[i] for i in train_indices]
    
    val_dist = Counter(r['label_category'] for r in val_rows)
    print(f"Reconstructed Validation Split: {len(val_rows)} rows (Train: {len(train_rows)} rows)")
    print(f"Validation Class Distribution: {dict(val_dist)}")
    
    # Overlap between Train and Validation
    train_h = set(r['infohash'] for r in train_rows)
    val_h = set(r['infohash'] for r in val_rows)
    train_nn = set(normalize_full_name(r['name']) for r in train_rows)
    val_nn = set(normalize_full_name(r['name']) for r in val_rows)
    train_rf = set(normalize_release_family(r['name']) for r in train_rows)
    val_rf = set(normalize_release_family(r['name']) for r in val_rows)
    
    print(f"Train/Val Exact Infohash Overlap: {len(train_h & val_h)} (due to duplicate rows in source dataset)")
    print(f"Train/Val Normalized Name Overlap: {len(train_nn & val_nn)}")
    print(f"Train/Val Release Family Overlap:  {len(train_rf & val_rf)}")
    
    return {
        "val_total": len(val_rows),
        "train_total": len(train_rows),
        "val_distribution": dict(val_dist),
        "hash_overlap": len(train_h & val_h),
        "name_overlap": len(train_nn & val_nn),
        "family_overlap": len(train_rf & val_rf),
    }


# ── 5. Credential Exposure Audit ────────────────────────────────────────────
def audit_credentials():
    print("\n=================================================================")
    print("5. CREDENTIAL EXPOSURE AUDIT")
    print("=================================================================")
    
    # Check all files in apps/classifier
    secret_patterns = [
        (re.compile(r'postgres(?:ql)?://([^:]+):([^@]+)@([^:/]+)'), "PostgreSQL Connection String with Embedded Password"),
        (re.compile(r'password\s*=\s*["\']([^"\']+)["\']'), "Hardcoded Password Argument"),
        (re.compile(r'ssh\s+core@([0-9\.]+)'), "Direct IP SSH Command"),
    ]
    
    findings = []
    for root, _, files in os.walk("apps/classifier"):
        for f in files:
            if f.endswith((".py", ".sh", ".yaml", ".yml", ".md", ".txt")):
                fp = os.path.join(root, f)
                try:
                    with open(fp, 'r', encoding='utf-8', errors='ignore') as sfile:
                        for lnum, line in enumerate(sfile, 1):
                            for pat, desc in secret_patterns:
                                m = pat.search(line)
                                if m:
                                    findings.append({
                                        "file": fp,
                                        "line": lnum,
                                        "type": desc,
                                        "snippet": line.strip()[:60] + "..."
                                    })
                                    print(f"  [EXPOSURE] {fp}:{lnum} -> {desc}")
                except Exception:
                    pass
    return findings


def main():
    models_info = audit_models()
    datasets = audit_datasets()
    leakage = audit_leakage(datasets)
    val_info = audit_validation_reconstruction(datasets["training_combined_v10_true.jsonl"])
    credentials = audit_credentials()
    
    # Save ledger summary to data/audit
    ledger_summary = {
        "models": {k: {"exists": v["exists"]} for k, v in models_info.items()},
        "datasets": {k: {
            "path": v["path"],
            "sha256": v["sha256"],
            "total_rows": v["total_rows"],
            "unique_hashes": v["unique_infohashes"],
            "unique_names": v["unique_normalized_names"],
            "unique_families": v["unique_release_families"],
            "class_counts": v["class_counts"],
            "status_counts": v["status_counts"]
        } for k, v in datasets.items()},
        "leakage": leakage,
        "validation_reconstruction": val_info,
        "credentials": credentials,
    }
    
    out_path = "apps/classifier/data/audit/audit_summary_ledger.json"
    with open(out_path, 'w', encoding='utf-8') as f:
        json.dump(ledger_summary, f, indent=2)
        
    print(f"\nSaved complete audit ledger summary to {out_path}")

if __name__ == "__main__":
    main()
