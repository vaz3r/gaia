#!/usr/bin/env python3
"""
Generate calibrated high-confidence pseudo-labels and uncertainty-sampled Active Learning candidates.
Uses per-class calibrated confidence thresholds based on empirical softmax percentiles.
"""
import argparse
import json
import logging
import time
from collections import defaultdict, Counter
from pathlib import Path

import joblib
import numpy as np
import yaml

from backends.transformer_onnx_backend import TransformerOnnxBackend
from core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

# Calibrated per-class thresholds (targeting top 10-20% cleanest predictions)
CLASS_THRESHOLDS = {
    "Other": 0.74,
    "Anime": 0.65,
    "Television": 0.58,
    "Movies": 0.50,
    "Applications": 0.61,
    "Games": 0.61,
    "Music": 0.58,
    "Documentaries": 0.58,
}

CLASS_CAPS = {
    "Other": 4000,
    "Movies": 2500,
    "Television": 2500,
    "Anime": 2000,
    "Games": 1000,
    "Applications": 1000,
    "Documentaries": 500,
    "Music": 500,
}

PSEUDO_SAMPLE_WEIGHT = 0.40


def compute_entropy(probs):
    eps = 1e-9
    p_safe = np.clip(probs, eps, 1.0)
    return -np.sum(p_safe * np.log2(p_safe), axis=-1)


def process_pool(
    input_pool_path="data/unlabeled_pool_60k.jsonl",
    pseudo_out_path="data/pseudo_labels_pool.jsonl",
    al_out_path="data/al_candidates_uncertain_1000.jsonl",
    batch_size=64,
    class_caps=None,
    class_thresholds=None,
    sample_weight=PSEUDO_SAMPLE_WEIGHT,
):
    caps = class_caps or CLASS_CAPS
    thresholds = class_thresholds or CLASS_THRESHOLDS
    
    config_path = Path("config/transformer.yaml")
    with open(config_path) as f:
        config = yaml.safe_load(f)
        
    model_path = Path("data/models/transformer/single_stage/model_int8.onnx")
    tokenizer_path = Path("data/models/transformer/single_stage/tokenizer")
    encoder_path = Path("data/models/transformer/single_stage/label_encoder.joblib")
    
    logger.info("Loading label encoder from %s...", encoder_path)
    le = joblib.load(encoder_path)
    classes = list(le.classes_)
    logger.info("Classes (%d): %s", len(classes), classes)
    
    logger.info("Initializing ONNX backend...")
    backend = TransformerOnnxBackend(
        model_path=str(model_path),
        tokenizer_path=str(tokenizer_path),
        max_length=256,
        num_threads=4,
    )
    
    torrents = []
    logger.info("Reading unlabeled pool from %s...", input_pool_path)
    with open(input_pool_path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            torrents.append(json.loads(line))
            
    n_total = len(torrents)
    logger.info("Loaded %d torrents for inference.", n_total)
    
    logger.info("Building text representations...")
    texts = [build_input_text(row, config) for row in torrents]
    
    logger.info("Running batched ONNX inference (batch_size=%d)...", batch_size)
    t0 = time.time()
    probs, predictions = backend.predict(texts, batch_size=batch_size)
    elapsed = time.time() - t0
    logger.info("Inference finished in %.1fs (throughput=%.1f it/s).", elapsed, n_total / max(1e-3, elapsed))
    
    entropies = compute_entropy(probs)
    sorted_probs = np.sort(probs, axis=1)[:, ::-1]
    top1_probs = sorted_probs[:, 0]
    top2_probs = sorted_probs[:, 1]
    margins = top1_probs - top2_probs
    
    logger.info("Per-class thresholds: %s", thresholds)
    logger.info("Per-class caps: %s", caps)
    
    by_class_candidates = defaultdict(list)
    for i in range(n_total):
        pred_idx = int(predictions[i])
        cat = classes[pred_idx]
        conf = float(top1_probs[i])
        thresh = thresholds.get(cat, 0.60)
        
        if conf >= thresh:
            by_class_candidates[cat].append((conf, i))
            
    selected_pseudo_indices = set()
    pseudo_records = []
    
    for cat, cand_list in by_class_candidates.items():
        cand_list.sort(key=lambda x: x[0], reverse=True)
        cap = caps.get(cat, 1000)
        chosen = cand_list[:cap]
        min_c = chosen[-1][0] if chosen else 0.0
        max_c = chosen[0][0] if chosen else 0.0
        logger.info("Category '%s': %d met threshold (>=%.2f), selected %d (conf: %.3f - %.3f)",
                    cat, len(cand_list), thresholds.get(cat, 0.60), len(chosen), min_c, max_c)
        
        for conf, idx in chosen:
            selected_pseudo_indices.add(idx)
            row = torrents[idx]
            pseudo_records.append({
                "infohash": row.get("infohash", row.get("id", "")),
                "name": row.get("name", ""),
                "file_count": row.get("file_count", 0),
                "total_size_bytes": row.get("total_size", row.get("total_size_bytes", 0)),
                "top_dirs": row.get("top_dirs", []),
                "label_category": cat,
                "confidence": round(conf, 4),
                "sample_weight": sample_weight,
                "is_pseudo": True,
            })
            
    logger.info("Total pseudo-labels selected: %d", len(pseudo_records))
    logger.info("Pseudo-label class distribution: %s", dict(Counter(r["label_category"] for r in pseudo_records)))
    
    with open(pseudo_out_path, "w", encoding="utf-8") as f_out:
        for r in pseudo_records:
            f_out.write(json.dumps(r) + "\n")
    logger.info("Saved pseudo-labels to %s", pseudo_out_path)
    
    # Uncertainty / Margin Active Learning candidate selection
    unlabeled_pool_indices = [i for i in range(n_total) if i not in selected_pseudo_indices]
    by_entropy = sorted(unlabeled_pool_indices, key=lambda i: entropies[i], reverse=True)
    by_margin = sorted(unlabeled_pool_indices, key=lambda i: margins[i])
    
    al_selected_set = set()
    for idx in by_entropy[:600]:
        if idx not in al_selected_set:
            al_selected_set.add(idx)
            
    for idx in by_margin[:600]:
        if idx not in al_selected_set:
            al_selected_set.add(idx)
            if len(al_selected_set) >= 1200:
                break
                
    al_records = []
    for idx in al_selected_set:
        row = torrents[idx]
        pred_idx = int(predictions[idx])
        top_idx = np.argsort(probs[idx])[::-1]
        top3 = [{"category": classes[k], "prob": round(float(probs[idx][k]), 4)} for k in top_idx[:3]]
        
        al_records.append({
            "infohash": row.get("infohash", row.get("id", "")),
            "name": row.get("name", ""),
            "file_count": row.get("file_count", 0),
            "total_size_bytes": row.get("total_size", row.get("total_size_bytes", 0)),
            "top_dirs": row.get("top_dirs", []),
            "model_pred": classes[pred_idx],
            "confidence": round(float(top1_probs[idx]), 4),
            "entropy": round(float(entropies[idx]), 4),
            "margin": round(float(margins[idx]), 4),
            "top3": top3,
        })
        
    logger.info("Selected %d high-uncertainty Active Learning candidates.", len(al_records))
    with open(al_out_path, "w", encoding="utf-8") as f_out:
        for r in al_records:
            f_out.write(json.dumps(r) + "\n")
    logger.info("Saved AL candidates to %s", al_out_path)
    
    return len(pseudo_records), len(al_records)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", default="data/unlabeled_pool_60k.jsonl")
    parser.add_argument("--pseudo-out", default="data/pseudo_labels_pool.jsonl")
    parser.add_argument("--al-out", default="data/al_candidates_uncertain_1000.jsonl")
    parser.add_argument("--sample-weight", type=float, default=PSEUDO_SAMPLE_WEIGHT)
    args = parser.parse_args()
    
    process_pool(
        input_pool_path=args.input,
        pseudo_out_path=args.pseudo_out,
        al_out_path=args.al_out,
        sample_weight=args.sample_weight,
    )
