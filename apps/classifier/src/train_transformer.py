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
from sklearn.metrics import accuracy_score, precision_recall_fscore_support
import joblib
from sklearn.utils.class_weight import compute_class_weight

from core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

# Reproducibility
SEED = 42
torch.manual_seed(SEED)
np.random.seed(SEED)
if torch.cuda.is_available():
    torch.cuda.manual_seed_all(SEED)

MAX_LENGTH = 256
BATCH_SIZE = 16

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

def load_data(path, label_encoder=None, fit_le=False):
    texts, labels = [], []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip(): continue
            row = json.loads(line)
            txt = build_input_text(row)
            if not txt.strip(): continue
            texts.append(txt)
            labels.append(row["label_category"])
            
    if fit_le:
        label_encoder = LabelEncoder()
        labels_encoded = label_encoder.fit_transform(labels)
        return texts, labels_encoded, label_encoder
    else:
        labels_encoded = label_encoder.transform(labels)
        return texts, labels_encoded

def evaluate(model, dataloader, device, label_encoder):
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
    p, r, f1, _ = precision_recall_fscore_support(all_labels, all_preds, average='macro', zero_division=0)
    return acc, f1

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", required=True, help="JSONL with true manual labels")
    parser.add_argument("--out_dir", default="data/models/transformer/single_stage")
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    logger.info("Loading true dataset...")
    true_texts, true_labels, le = load_data(args.data, fit_le=True)
    joblib.dump(le, out_dir / "label_encoder.joblib")
    
    from sklearn.model_selection import train_test_split
    t_train_texts, t_val_texts, t_train_labels, t_val_labels = train_test_split(
        true_texts, true_labels, test_size=0.2, stratify=true_labels, random_state=42
    )

    logger.info("Loading tokenizer...")
    tokenizer = AutoTokenizer.from_pretrained("sentence-transformers/all-MiniLM-L12-v2")
    tokenizer.save_pretrained(str(out_dir / "tokenizer"))

    logger.info("Encoding true datasets...")
    t_train_enc = tokenizer(t_train_texts, truncation=True, padding="max_length", max_length=MAX_LENGTH)
    t_val_enc = tokenizer(t_val_texts, truncation=True, padding="max_length", max_length=MAX_LENGTH)

    t_train_ds = TorrentDataset(t_train_enc, t_train_labels)
    t_val_ds = TorrentDataset(t_val_enc, t_val_labels)
    t_val_dl = DataLoader(t_val_ds, batch_size=BATCH_SIZE)
    t_train_dl = DataLoader(t_train_ds, batch_size=BATCH_SIZE, shuffle=True)

    # Moderate Capped Class Weights
    weights = compute_class_weight('balanced', classes=np.unique(t_train_labels), y=t_train_labels)
    weights = np.clip(weights, 0.5, 3.0)
    
    # Gently increase weight for 'Other' to improve recall
    if 'Other' in le.classes_:
        other_idx = list(le.classes_).index('Other')
        weights[other_idx] = weights[other_idx] * 1.2
        
    class_weights_tensor = torch.tensor(weights, dtype=torch.float32)

    model = AutoModelForSequenceClassification.from_pretrained(
        "sentence-transformers/all-MiniLM-L12-v2", num_labels=len(le.classes_)
    )
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model.to(device)
    class_weights_tensor = class_weights_tensor.to(device)

    logger.info("=== Phase: True Fine-tuning (4 Epochs) ===")
    optimizer = AdamW(model.parameters(), lr=2e-5)
    epochs = 4
    scheduler = get_linear_schedule_with_warmup(optimizer, num_warmup_steps=len(t_train_dl)//10, num_training_steps=len(t_train_dl)*epochs)
    
    best_f1 = 0
    for epoch in range(epochs):
        model.train()
        total_loss = 0
        for step, batch in enumerate(t_train_dl):
            optimizer.zero_grad()
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels = batch["labels"].to(device)
            
            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            loss_fct = torch.nn.CrossEntropyLoss(weight=class_weights_tensor)
            loss = loss_fct(outputs.logits.view(-1, len(le.classes_)), labels.view(-1))
            
            loss.backward()
            optimizer.step()
            scheduler.step()
            total_loss += loss.item()
            
        acc, f1 = evaluate(model, t_val_dl, device, le)
        logger.info(f"Epoch {epoch+1} Validation: Acc={acc:.3f}, MacroF1={f1:.3f}")
        if f1 > best_f1:
            best_f1 = f1
            model.save_pretrained(out_dir / "model")
            logger.info("Saved best model")

if __name__ == "__main__":
    main()
