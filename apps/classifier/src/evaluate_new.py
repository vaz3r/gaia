#!/usr/bin/env python3
"""Evaluate the new transformer model on benchmark sets."""

import json
import logging
import sys
from pathlib import Path

import numpy as np
import torch
import joblib
from sklearn.metrics import accuracy_score, precision_recall_fscore_support, confusion_matrix
from transformers import AutoModelForSequenceClassification, AutoTokenizer

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from src.core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger("evaluate_new")


def load_benchmark(path: str) -> tuple[list[dict], list[str]]:
    """Load benchmark JSONL, return (torrents, labels)."""
    torrents, labels = [], []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            torrents.append(row)
            labels.append(row.get("label_category", row.get("true_category", "")))
    return torrents, labels


def evaluate(model_dir: str, benchmark_path: str, max_length: int = 256):
    """Evaluate model on benchmark."""
    model_dir = Path(model_dir)

    # Load model, tokenizer, label encoder
    logger.info(f"Loading model from {model_dir}")
    tokenizer = AutoTokenizer.from_pretrained(str(model_dir / "tokenizer"))
    model = AutoModelForSequenceClassification.from_pretrained(str(model_dir / "model"))
    le = joblib.load(model_dir / "label_encoder.joblib")
    classes = le.classes_

    device = torch.device("cuda" if torch.cuda.is_available() else ("mps" if torch.backends.mps.is_available() else "cpu"))
    model.to(device)
    model.eval()
    logger.info(f"Device: {device} | Classes: {list(classes)}")

    # Load benchmark
    torrents, gold_labels = load_benchmark(benchmark_path)
    logger.info(f"Loaded {len(torrents)} samples from {benchmark_path}")

    # Build texts
    texts = [build_input_text(t) for t in torrents]

    # Tokenize
    encodings = tokenizer(texts, truncation=True, padding="max_length", max_length=max_length)

    # Predict in batches
    all_preds = []
    batch_size = 32
    with torch.no_grad():
        for i in range(0, len(texts), batch_size):
            input_ids = torch.tensor(encodings["input_ids"][i:i+batch_size]).to(device)
            attention_mask = torch.tensor(encodings["attention_mask"][i:i+batch_size]).to(device)
            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            preds = torch.argmax(outputs.logits, dim=1).cpu().numpy()
            all_preds.extend(preds)

    pred_labels = [classes[p] for p in all_preds]

    # Metrics
    acc = accuracy_score(gold_labels, pred_labels)
    p, r, f1, support = precision_recall_fscore_support(gold_labels, pred_labels, average="macro", zero_division=0)
    p_per, r_per, f1_per, support_per = precision_recall_fscore_support(gold_labels, pred_labels, average=None, zero_division=0, labels=sorted(set(gold_labels)))

    print(f"\n{'='*60}")
    print(f"Benchmark: {benchmark_path}")
    print(f"Samples:   {len(torrents)}")
    print(f"Accuracy:  {acc:.3f} ({int(acc*len(torrents))}/{len(torrents)})")
    print(f"Macro F1:  {f1:.3f}")
    print(f"{'='*60}")

    print(f"\n{'Category':<20} {'Precision':>9} {'Recall':>9} {'F1':>9} {'Support':>9}")
    print("-" * 56)
    for i, cat in enumerate(sorted(set(gold_labels))):
        print(f"{cat:<20} {p_per[i]:>9.3f} {r_per[i]:>9.3f} {f1_per[i]:>9.3f} {support_per[i]:>9}")

    # Confusion matrix
    cats = sorted(set(gold_labels))
    cm = confusion_matrix(gold_labels, pred_labels, labels=cats)
    print(f"\nConfusion matrix (gold=rows, pred=cols):")
    header = f"{'Gold\\Pred':<20}" + "".join(f"{c[:10]:>11}" for c in cats)
    print(header)
    for i, cat in enumerate(cats):
        row = f"{cat:<20}" + "".join(f"{cm[i][j]:>11}" for j in range(len(cats)))
        print(row)

    return acc, f1


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--model_dir", default="data/models/transformer_v2")
    parser.add_argument("--benchmark", default="data/manual_eval_set_balanced_2000.jsonl")
    args = parser.parse_args()

    # If benchmark not found, use test split from exported data
    benchmark = args.benchmark
    if not Path(benchmark).exists():
        benchmark = "data/deepseek_labeled/test.jsonl"
        logger.warning(f"Benchmark not found, using test split: {benchmark}")

    evaluate(args.model_dir, benchmark)
