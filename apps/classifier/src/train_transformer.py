#!/usr/bin/env python3
"""
Train a transformer classifier for torrent metadata.

Supports two modes:
  - Full mode (default): text_builder with regex feature detection
  - Raw mode: metadata only (name, file_count, size, extensions, folders)

Usage:
    # Full mode (legacy training data)
    python src/train_transformer.py --data data/training_combined_v10_true.jsonl

    # Raw mode (Gemini-labeled data)
    python src/train_transformer.py --data data/gemini_labeled/train.jsonl --config config/transformer_v2.yaml
"""

import argparse
import json
import logging
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import DataLoader, Dataset
from transformers import AutoModelForSequenceClassification, AutoTokenizer
from transformers import get_linear_schedule_with_warmup
from torch.optim import AdamW
from sklearn.preprocessing import LabelEncoder
from sklearn.model_selection import train_test_split
from sklearn.metrics import accuracy_score, precision_recall_fscore_support
import joblib
from sklearn.utils.class_weight import compute_class_weight

from core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

SEED = 42
torch.manual_seed(SEED)
np.random.seed(SEED)
if torch.cuda.is_available():
    torch.cuda.manual_seed_all(SEED)
if torch.backends.mps.is_available():
    torch.mps.manual_seed(SEED)


class TorrentDataset(Dataset):
    def __init__(self, encodings, labels):
        self.encodings = encodings
        self.labels = labels

    def __getitem__(self, idx):
        item = {key: torch.tensor(val[idx]) for key, val in self.encodings.items()}
        item["labels"] = torch.tensor(self.labels[idx], dtype=torch.long)
        return item

    def __len__(self):
        return len(self.labels)


def load_data(path: str, config: dict | None = None) -> tuple[list[str], np.ndarray, LabelEncoder]:
    """Load JSONL data and build text representations."""
    texts, labels = [], []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            row = json.loads(line)
            txt = build_input_text(row, config)
            if not txt.strip():
                continue
            texts.append(txt)
            labels.append(row["label_category"])

    le = LabelEncoder()
    labels_encoded = le.fit_transform(labels)
    return texts, labels_encoded, le


def evaluate(model, dataloader, device, n_classes: int):
    model.eval()
    all_preds, all_labels = [], []
    with torch.no_grad():
        for batch in dataloader:
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels = batch["labels"].to(device)

            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            preds = torch.argmax(outputs.logits, dim=1)
            all_preds.extend(preds.cpu().numpy())
            all_labels.extend(labels.cpu().numpy())

    acc = accuracy_score(all_labels, all_preds)
    p, r, f1, _ = precision_recall_fscore_support(all_labels, all_preds, average="macro", zero_division=0)
    return acc, f1


def main():
    parser = argparse.ArgumentParser(description="Train torrent classifier")
    parser.add_argument("--data", required=True, help="JSONL training data")
    parser.add_argument("--out_dir", default="data/models/transformer_v2", help="Output directory")
    parser.add_argument("--config", default=None, help="Config YAML (for raw mode)")
    parser.add_argument("--epochs", type=int, default=None, help="Override epochs")
    parser.add_argument("--lr", type=float, default=None, help="Override learning rate")
    parser.add_argument("--val_ratio", type=float, default=0.15, help="Validation split ratio")
    args = parser.parse_args()

    # Load config
    config = {}
    if args.config:
        import yaml
        with open(args.config) as f:
            config = yaml.safe_load(f)

    tr_cfg = config.get("transformer", {})
    epochs = args.epochs or tr_cfg.get("epochs", 4)
    lr = args.lr or tr_cfg.get("learning_rate", 2e-5)
    batch_size = tr_cfg.get("batch_size", 16)
    max_length = tr_cfg.get("max_length", 256)
    grad_clip = tr_cfg.get("grad_clip", 1.0)

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    # Load data
    logger.info("Loading dataset from %s...", args.data)
    texts, labels, le = load_data(args.data, config)
    joblib.dump(le, out_dir / "label_encoder.joblib")

    n_total = len(texts)
    n_classes = len(le.classes_)
    logger.info("Total samples: %d | Classes: %d | %s", n_total, n_classes, list(le.classes_))

    # Stratified train/val split
    train_texts, val_texts, train_labels, val_labels = train_test_split(
        texts, labels, test_size=args.val_ratio, stratify=labels, random_state=SEED
    )
    logger.info("Train: %d | Val: %d", len(train_texts), len(val_texts))

    # Tokenize
    logger.info("Loading tokenizer...")
    tokenizer = AutoTokenizer.from_pretrained("sentence-transformers/all-MiniLM-L12-v2")
    tokenizer.save_pretrained(str(out_dir / "tokenizer"))

    logger.info("Encoding datasets...")
    train_enc = tokenizer(train_texts, truncation=True, padding="max_length", max_length=max_length)
    val_enc = tokenizer(val_texts, truncation=True, padding="max_length", max_length=max_length)

    train_ds = TorrentDataset(train_enc, train_labels)
    val_ds = TorrentDataset(val_enc, val_labels)
    train_dl = DataLoader(train_ds, batch_size=batch_size, shuffle=True)
    val_dl = DataLoader(val_ds, batch_size=batch_size)

    # Class weights (balanced, capped)
    weights = compute_class_weight("balanced", classes=np.unique(train_labels), y=train_labels)
    weights = np.clip(weights, 0.5, 3.0)
    class_weights_tensor = torch.tensor(weights, dtype=torch.float32)

    # Model
    model = AutoModelForSequenceClassification.from_pretrained(
        "sentence-transformers/all-MiniLM-L12-v2", num_labels=n_classes
    )
    device = torch.device("cuda" if torch.cuda.is_available() else ("mps" if torch.backends.mps.is_available() else "cpu"))
    logger.info("Device: %s", device)
    model.to(device)
    class_weights_tensor = class_weights_tensor.to(device)

    # Optimizer + scheduler
    optimizer = AdamW(model.parameters(), lr=lr, weight_decay=tr_cfg.get("weight_decay", 0.01))
    warmup_steps = len(train_dl) // 10
    total_steps = len(train_dl) * epochs
    scheduler = get_linear_schedule_with_warmup(optimizer, num_warmup_steps=warmup_steps, num_training_steps=total_steps)

    # Training loop
    logger.info("=== Training (%d epochs, lr=%.1e) ===", epochs, lr)
    best_f1 = 0
    for epoch in range(epochs):
        model.train()
        total_loss = 0
        for batch in train_dl:
            optimizer.zero_grad()
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels_tensor = batch["labels"].to(device)

            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            loss_fct = torch.nn.CrossEntropyLoss(weight=class_weights_tensor)
            loss = loss_fct(outputs.logits, labels_tensor)

            loss.backward()
            if grad_clip > 0:
                torch.nn.utils.clip_grad_norm_(model.parameters(), grad_clip)
            optimizer.step()
            scheduler.step()
            total_loss += loss.item()

        avg_loss = total_loss / len(train_dl)
        acc, f1 = evaluate(model, val_dl, device, n_classes)
        logger.info("Epoch %d/%d — loss=%.4f  val_acc=%.3f  val_macro_f1=%.3f", epoch + 1, epochs, avg_loss, acc, f1)

        if f1 > best_f1:
            best_f1 = f1
            model.save_pretrained(out_dir / "model")
            logger.info("Saved best model (macro_f1=%.4f)", best_f1)

    logger.info("Training complete. Best val macro_f1=%.4f", best_f1)
    logger.info("Model saved to: %s", out_dir / "model")


if __name__ == "__main__":
    main()
