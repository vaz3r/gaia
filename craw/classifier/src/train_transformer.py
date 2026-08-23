#!/usr/bin/env python3
"""Fine-tune DistilBERT for torrent classification on Mac M1 (MPS)."""

import sys
import json
import time
import logging
from pathlib import Path

logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='%(asctime)s %(levelname)s %(message)s')
logger = logging.getLogger('train_transformer')

import numpy as np
import joblib
import torch
from torch.utils.data import Dataset, DataLoader
from transformers import (
    AutoTokenizer,
    AutoModelForSequenceClassification,
    get_linear_schedule_with_warmup,
)
from sklearn.model_selection import train_test_split
from sklearn.preprocessing import LabelEncoder
from sklearn.utils.class_weight import compute_class_weight

sys.path.insert(0, '.')
from src.core.text_builder import build_input_text
from src.core.types import ALLOWED_CATEGORIES

# ── Config ──────────────────────────────────────────────────────────────────

MODEL_NAME = "distilbert-base-uncased"
MAX_LENGTH = 128
BATCH_SIZE = 8
NUM_EPOCHS = 8
LR = 3e-5
WARMUP_RATIO = 0.1
WEIGHT_DECAY = 0.01
SEED = 42
OUT_DIR = Path("data/models/transformer")

# ── Dataset ─────────────────────────────────────────────────────────────────

class TorrentDataset(Dataset):
    def __init__(self, texts, labels, tokenizer, max_length):
        self.encodings = tokenizer(
            texts,
            truncation=True,
            padding="max_length",
            max_length=max_length,
            return_tensors="pt",
        )
        self.labels = torch.tensor(labels, dtype=torch.long)

    def __len__(self):
        return len(self.labels)

    def __getitem__(self, idx):
        item = {k: v[idx] for k, v in self.encodings.items()}
        item["labels"] = self.labels[idx]
        return item


# ── Main ────────────────────────────────────────────────────────────────────

def main():
    torch.manual_seed(SEED)
    np.random.seed(SEED)

    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")
    logger.info("Device: %s", device)

    # Load data
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", default="data/labeled_augmented.jsonl", help="Training data JSONL")
    args, _ = parser.parse_known_args()

    logger.info("Loading labeled data from %s...", args.data)
    records = []
    with open(args.data) as f:
        for line in f:
            if line.strip():
                records.append(json.loads(line))
    logger.info("Loaded %d records", len(records))

    texts = [build_input_text(r) for r in records]
    raw_labels = [r["label_category"] for r in records]

    # Encode labels
    le = LabelEncoder()
    labels = le.fit_transform(raw_labels)
    num_labels = len(le.classes_)
    logger.info("Classes (%d): %s", num_labels, list(le.classes_))

    # Class distribution
    unique, counts = np.unique(labels, return_counts=True)
    for idx, cnt in zip(unique, counts):
        logger.info("  %s: %d (%.1f%%)", le.classes_[idx], cnt, cnt / len(labels) * 100)

    # Compute class weights for imbalanced data
    class_weights = compute_class_weight("balanced", classes=np.arange(num_labels), y=labels)
    class_weights_tensor = torch.tensor(class_weights, dtype=torch.float32).to(device)
    logger.info("Class weights: %s", {le.classes_[i]: f"{w:.2f}" for i, w in enumerate(class_weights)})

    # Split
    X_train, X_val, y_train, y_val = train_test_split(
        texts, labels, test_size=0.2, random_state=SEED, stratify=labels
    )
    logger.info("Split: train=%d val=%d", len(X_train), len(X_val))

    # Tokenizer + model
    logger.info("Loading %s...", MODEL_NAME)
    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME)
    model = AutoModelForSequenceClassification.from_pretrained(
        MODEL_NAME, num_labels=num_labels
    ).to(device)
    logger.info("Model params: %d", sum(p.numel() for p in model.parameters()))

    # Datasets + loaders
    train_dataset = TorrentDataset(X_train, y_train, tokenizer, MAX_LENGTH)
    val_dataset = TorrentDataset(X_val, y_val, tokenizer, MAX_LENGTH)

    train_loader = DataLoader(train_dataset, batch_size=BATCH_SIZE, shuffle=True)
    val_loader = DataLoader(val_dataset, batch_size=BATCH_SIZE * 2)

    # Optimizer + scheduler
    no_decay = ["bias", "LayerNorm.weight"]
    optimizer_grouped_params = [
        {
            "params": [p for n, p in model.named_parameters() if not any(nd in n for nd in no_decay)],
            "weight_decay": WEIGHT_DECAY,
        },
        {
            "params": [p for n, p in model.named_parameters() if any(nd in n for nd in no_decay)],
            "weight_decay": 0.0,
        },
    ]
    optimizer = torch.optim.AdamW(optimizer_grouped_params, lr=LR)
    total_steps = len(train_loader) * NUM_EPOCHS
    warmup_steps = int(total_steps * WARMUP_RATIO)
    scheduler = get_linear_schedule_with_warmup(optimizer, num_warmup_steps=warmup_steps, num_training_steps=total_steps)

    logger.info("Training: %d epochs, %d steps, %d warmup steps", NUM_EPOCHS, total_steps, warmup_steps)

    # Training loop
    best_val_f1 = 0.0
    for epoch in range(NUM_EPOCHS):
        # Train
        model.train()
        total_loss = 0.0
        t0 = time.time()
        for batch_idx, batch in enumerate(train_loader):
            batch = {k: v.to(device) for k, v in batch.items()}
            outputs = model(**batch)

            # Weighted loss
            loss_fct = torch.nn.CrossEntropyLoss(weight=class_weights_tensor)
            logits = outputs.logits
            loss = loss_fct(logits, batch["labels"])

            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            scheduler.step()
            optimizer.zero_grad()

            total_loss += loss.item()
            if (batch_idx + 1) % 20 == 0:
                logger.info("  [%d/%d] loss=%.4f", batch_idx + 1, len(train_loader), total_loss / (batch_idx + 1))

        avg_loss = total_loss / len(train_loader)
        train_time = time.time() - t0

        # Validate
        model.eval()
        all_preds = []
        all_labels = []
        with torch.no_grad():
            for batch in val_loader:
                batch = {k: v.to(device) for k, v in batch.items()}
                outputs = model(**batch)
                preds = torch.argmax(outputs.logits, dim=-1)
                all_preds.extend(preds.cpu().numpy())
                all_labels.extend(batch["labels"].cpu().numpy())

        all_preds = np.array(all_preds)
        all_labels = np.array(all_labels)
        accuracy = (all_preds == all_labels).mean()

        # Per-class F1
        from sklearn.metrics import classification_report, f1_score
        report = classification_report(
            all_labels, all_preds,
            target_names=le.classes_,
            zero_division=0,
            output_dict=True,
        )
        macro_f1 = report["macro avg"]["f1-score"]

        logger.info(
            "Epoch %d/%d: loss=%.4f acc=%.3f macro_f1=%.3f time=%.1fs",
            epoch + 1, NUM_EPOCHS, avg_loss, accuracy, macro_f1, train_time,
        )
        logger.info("\n%s", classification_report(all_labels, all_preds, target_names=le.classes_, zero_division=0))

        # Save best
        if macro_f1 > best_val_f1:
            best_val_f1 = macro_f1
            save_dir = OUT_DIR
            save_dir.mkdir(parents=True, exist_ok=True)

            model.save_pretrained(str(save_dir / "model"))
            tokenizer.save_pretrained(str(save_dir / "tokenizer"))
            joblib.dump(le, save_dir / "label_encoder.joblib")

            with open(save_dir / "training_report.txt", "w") as f:
                f.write(classification_report(all_labels, all_preds, target_names=le.classes_, zero_division=0))

            logger.info("Saved best model (macro_f1=%.3f)", best_val_f1)

    logger.info("Done. Best macro_f1=%.3f", best_val_f1)


if __name__ == "__main__":
    main()
