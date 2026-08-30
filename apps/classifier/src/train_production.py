#!/usr/bin/env python3
"""
Production Classifier Training and Multi-Benchmark Evaluation Pipeline.
Trains a high-precision sequence classification transformer on the unified 17,000+ record dataset,
applying balanced loss weighting, text feature extraction, and evaluates across:
1. Reference Gold Pilot v1 (300 rows)
2. Balanced Benchmark (1,650 rows)
3. Natural Skew Benchmark (1,000 rows)
4. Frozen Validation Split (627 rows)
"""
from __future__ import annotations

import argparse
import json
import logging
import os
import random
import sys
from collections import Counter
from pathlib import Path

import numpy as np
import sklearn
from sklearn.metrics import (
    accuracy_score,
    confusion_matrix,
    f1_score,
    precision_recall_fscore_support,
)
from sklearn.preprocessing import LabelEncoder
from sklearn.utils.class_weight import compute_class_weight
import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.optim import AdamW
from torch.utils.data import DataLoader, Dataset
import transformers
from transformers import (
    AutoConfig,
    AutoModelForSequenceClassification,
    AutoTokenizer,
    get_cosine_schedule_with_warmup,
)

# Core text builder import
apps_dir = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(apps_dir / "src"))
from core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("train_production")

FIXED_CLASSES = [
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
]

ID2LABEL = {i: c for i, c in enumerate(FIXED_CLASSES)}
LABEL2ID = {c: i for i, c in enumerate(FIXED_CLASSES)}

SEED = 42
MAX_LENGTH = 256
BATCH_SIZE = 16
LEARNING_RATE = 2.5e-5
WEIGHT_DECAY = 0.01
EPOCHS = 5
WARMUP_RATIO = 0.10
MAX_GRAD_NORM = 1.0


def set_seed(seed: int = SEED):
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    if torch.backends.mps.is_available():
        torch.mps.manual_seed(seed)


def get_device() -> torch.device:
    if torch.cuda.is_available():
        return torch.device("cuda")
    if torch.backends.mps.is_available():
        return torch.device("mps")
    return torch.device("cpu")


class TorrentDataset(Dataset):
    def __init__(self, encodings, labels, sample_weights):
        self.encodings = encodings
        self.labels = labels
        self.sample_weights = sample_weights

    def __len__(self) -> int:
        return len(self.labels)

    def __getitem__(self, idx: int) -> dict[str, torch.Tensor]:
        item = {k: torch.tensor(v[idx], dtype=torch.long) for k, v in self.encodings.items()}
        item["labels"] = torch.tensor(self.labels[idx], dtype=torch.long)
        item["sample_weight"] = torch.tensor(self.sample_weights[idx], dtype=torch.float32)
        return item


def load_dataset_rows(path: Path) -> list[dict]:
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not (line.startswith("{") and line.endswith("}")):
                continue
            rows.append(json.loads(line))
    return rows


def build_training_pool() -> tuple[list[str], list[int], list[float]]:
    """
    Merge pooled labeled data + subagent annotations, balance representation,
    and format with text_builder.
    """
    raw_rows = []
    
    labeled_path = Path("data/production_pool/pooled_labeled.jsonl")
    if labeled_path.exists():
        raw_rows.extend(load_dataset_rows(labeled_path))
        
    annotated_path = Path("data/production_pool/subagent_annotated.jsonl")
    if annotated_path.exists():
        raw_rows.extend(load_dataset_rows(annotated_path))

    # Also include baseline_v2 training set
    bv2_train = Path("data/baseline_v2/train.jsonl")
    if bv2_train.exists():
        raw_rows.extend(load_dataset_rows(bv2_train))

    seen_ih = set()
    deduped_rows = []
    for r in raw_rows:
        ih = r.get("infohash", "")
        if ih and ih in seen_ih:
            continue
        if ih:
            seen_ih.add(ih)
        cat = r.get("label_category")
        if cat in LABEL2ID:
            deduped_rows.append(r)

    logger.info("Total deduplicated training records: %d", len(deduped_rows))
    class_counts = Counter(r["label_category"] for r in deduped_rows)
    logger.info("Class distribution before balancing: %s", dict(class_counts))

    # Balance under-represented classes by oversampling them to target min count
    target_count = 1500
    by_class: dict[str, list[dict]] = {c: [] for c in FIXED_CLASSES}
    for r in deduped_rows:
        by_class[r["label_category"]].append(r)

    balanced_rows = []
    for c, items in by_class.items():
        if not items:
            continue
        if len(items) < target_count:
            # Oversample to reach target_count
            multiplier = (target_count // len(items)) + 1
            expanded = (items * multiplier)[:target_count]
            balanced_rows.extend(expanded)
        else:
            # Cap majority classes to prevent domination
            balanced_rows.extend(items[:target_count * 2])

    random.seed(SEED)
    random.shuffle(balanced_rows)
    logger.info("Total balanced training dataset size: %d", len(balanced_rows))
    logger.info("Class distribution after balancing: %s", dict(Counter(r["label_category"] for r in balanced_rows)))

    texts = [build_input_text(r) for r in balanced_rows]
    labels = [LABEL2ID[r["label_category"]] for r in balanced_rows]
    weights = [float(r.get("sample_weight", 1.0)) for r in balanced_rows]

    return texts, labels, weights


def evaluate_model(model, tokenizer, dataset_path: Path, device: torch.device, name: str = "Benchmark") -> dict:
    if not dataset_path.exists():
        logger.warning("Dataset path %s does not exist, skipping eval.", dataset_path)
        return {}

    rows = load_dataset_rows(dataset_path)
    if not rows:
        return {}

    texts = []
    y_true = []
    valid_rows = []
    for r in rows:
        label = r.get("label_category", r.get("TRUE_LABEL", None))
        if label not in LABEL2ID:
            continue
        texts.append(build_input_text(r))
        y_true.append(LABEL2ID[label])
        valid_rows.append(r)

    encodings = tokenizer(
        texts,
        truncation=True,
        padding="max_length",
        max_length=MAX_LENGTH,
        return_tensors="pt",
    )

    dataset = TorrentDataset(
        encodings={k: v.numpy() for k, v in encodings.items()},
        labels=np.array(y_true),
        sample_weights=np.ones(len(y_true), dtype=np.float32),
    )
    dataloader = DataLoader(dataset, batch_size=32, shuffle=False)

    model.eval()
    all_preds = []
    all_probs = []
    with torch.no_grad():
        for batch in dataloader:
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            probs = F.softmax(outputs.logits, dim=1).cpu().numpy()
            preds = np.argmax(probs, axis=1)
            all_preds.extend(preds)
            all_probs.extend(probs)

    y_pred = np.array(all_preds)
    acc = accuracy_score(y_true, y_pred)
    macro_p, macro_r, macro_f1, _ = precision_recall_fscore_support(y_true, y_pred, average="macro", zero_division=0)
    weighted_p, weighted_r, weighted_f1, _ = precision_recall_fscore_support(y_true, y_pred, average="weighted", zero_division=0)
    p_per, r_per, f1_per, sup_per = precision_recall_fscore_support(y_true, y_pred, labels=list(range(len(FIXED_CLASSES))), zero_division=0)

    per_class_metrics = {}
    for idx, c in enumerate(FIXED_CLASSES):
        per_class_metrics[c] = {
            "precision": float(p_per[idx]),
            "recall": float(r_per[idx]),
            "f1": float(f1_per[idx]),
            "support": int(sup_per[idx]),
        }

    logger.info("=== %s Results (N=%d) ===", name, len(y_true))
    logger.info("Overall Accuracy: %.2f%% | Macro F1: %.4f | Weighted F1: %.4f", acc * 100, macro_f1, weighted_f1)
    for c in FIXED_CLASSES:
        m = per_class_metrics[c]
        logger.info("  %-15s | P: %6.2f%% | R: %6.2f%% | F1: %6.2f%% | Support: %4d", c, m["precision"]*100, m["recall"]*100, m["f1"]*100, m["support"])

    return {
        "benchmark_name": name,
        "support": len(y_true),
        "accuracy": float(acc),
        "macro_f1": float(macro_f1),
        "weighted_f1": float(weighted_f1),
        "per_class": per_class_metrics,
        "confusion_matrix": confusion_matrix(y_true, y_pred, labels=list(range(len(FIXED_CLASSES)))).tolist(),
    }


def train_and_evaluate(model_name: str = "sentence-transformers/all-MiniLM-L12-v2", epochs: int = EPOCHS):
    set_seed(SEED)
    device = get_device()
    logger.info("Using device: %s", device)

    out_dir = Path("data/models/transformer/production_v1")
    out_dir.mkdir(parents=True, exist_ok=True)

    tokenizer = AutoTokenizer.from_pretrained(model_name)
    config = AutoConfig.from_pretrained(
        model_name,
        num_labels=len(FIXED_CLASSES),
        id2label=ID2LABEL,
        label2id=LABEL2ID,
    )
    model = AutoModelForSequenceClassification.from_pretrained(model_name, config=config)
    model.to(device)

    train_texts, train_labels, train_weights = build_training_pool()
    train_encodings = tokenizer(
        train_texts,
        truncation=True,
        padding="max_length",
        max_length=MAX_LENGTH,
        return_tensors="pt",
    )

    # Compute class weights to ensure minority classes are penalized properly
    cls_weights = compute_class_weight(
        class_weight="balanced",
        classes=np.arange(len(FIXED_CLASSES)),
        y=np.array(train_labels),
    )
    class_weights_t = torch.tensor(cls_weights, dtype=torch.float32).to(device)
    logger.info("Computed class weights: %s", {FIXED_CLASSES[i]: round(float(w), 3) for i, w in enumerate(cls_weights)})

    train_dataset = TorrentDataset(
        encodings={k: v.numpy() for k, v in train_encodings.items()},
        labels=np.array(train_labels),
        sample_weights=np.array(train_weights),
    )
    train_loader = DataLoader(train_dataset, batch_size=BATCH_SIZE, shuffle=True)

    optimizer = AdamW(model.parameters(), lr=LEARNING_RATE, weight_decay=WEIGHT_DECAY)
    total_steps = len(train_loader) * epochs
    warmup_steps = int(total_steps * WARMUP_RATIO)
    scheduler = get_cosine_schedule_with_warmup(optimizer, num_warmup_steps=warmup_steps, num_training_steps=total_steps)

    val_path = Path("data/baseline_v2/validation.jsonl")
    best_val_f1 = 0.0

    for epoch in range(1, epochs + 1):
        model.train()
        total_loss = 0.0
        for batch_idx, batch in enumerate(train_loader):
            optimizer.zero_grad()
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels = batch["labels"].to(device)
            sample_weight = batch["sample_weight"].to(device)

            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            logits = outputs.logits

            # Weighted cross-entropy with sample weight
            unweighted_loss = F.cross_entropy(logits, labels, weight=class_weights_t, reduction="none")
            loss = (unweighted_loss * sample_weight).mean()

            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), MAX_GRAD_NORM)
            optimizer.step()
            scheduler.step()

            total_loss += loss.item()

        avg_loss = total_loss / len(train_loader)
        logger.info("Epoch %d/%d - Average Training Loss: %.4f", epoch, epochs, avg_loss)

        # Validate on frozen validation set
        val_results = evaluate_model(model, tokenizer, val_path, device, name=f"Validation (Epoch {epoch})")
        val_f1 = val_results.get("macro_f1", 0.0)

        if val_f1 > best_val_f1 or epoch == epochs:
            best_val_f1 = max(best_val_f1, val_f1)
            logger.info(">>> Saving best checkpoint to %s (Val Macro F1: %.4f)", out_dir / "model", val_f1)
            model.save_pretrained(out_dir / "model")
            tokenizer.save_pretrained(out_dir / "tokenizer")

    # Load best model for full benchmark evaluation
    logger.info("Loading best model for multi-benchmark evaluation...")
    best_model = AutoModelForSequenceClassification.from_pretrained(out_dir / "model")
    best_model.to(device)

    benchmarks = {
        "Reference Gold Pilot v1 (300 rows)": Path("data/gold_pilot_v1/reference_eval_v1.jsonl"),
        "Balanced Benchmark (1650 rows)": Path("data/manual_eval_set_balanced_2000.jsonl"),
        "Natural Skew Benchmark (1000 rows)": Path("data/manual_eval_set_1000.jsonl"),
        "Grouped Validation Split (627 rows)": Path("data/baseline_v2/validation.jsonl"),
    }

    all_metrics = {}
    for name, bpath in benchmarks.items():
        res = evaluate_model(best_model, tokenizer, bpath, device, name=name)
        all_metrics[name] = res

    with open(out_dir / "final_evaluation_report.json", "w", encoding="utf-8") as f:
        json.dump(all_metrics, f, indent=2)
    logger.info("Saved final multi-benchmark evaluation report to %s", out_dir / "final_evaluation_report.json")


if __name__ == "__main__":
    train_and_evaluate()
