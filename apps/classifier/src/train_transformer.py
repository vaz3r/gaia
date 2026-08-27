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
if torch.backends.mps.is_available():
    torch.mps.manual_seed(SEED)

MAX_LENGTH = 256
BATCH_SIZE = 16

class TorrentDataset(Dataset):
    def __init__(self, encodings, labels, sample_weights=None):
        self.encodings = encodings
        self.labels = labels
        if sample_weights is None:
            self.sample_weights = np.ones(len(labels), dtype=np.float32)
        else:
            self.sample_weights = np.array(sample_weights, dtype=np.float32)

    def __getitem__(self, idx):
        item = {key: torch.tensor(val[idx]) for key, val in self.encodings.items()}
        item["labels"] = torch.tensor(self.labels[idx], dtype=torch.long)
        item["sample_weights"] = torch.tensor(self.sample_weights[idx], dtype=torch.float32)
        return item

    def __len__(self):
        return len(self.labels)

def load_data(path, label_encoder=None, fit_le=False):
    texts, labels, sample_weights, is_pseudo = [], [], [], []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip(): continue
            row = json.loads(line)
            txt = build_input_text(row)
            if not txt.strip(): continue
            texts.append(txt)
            labels.append(row["label_category"])
            weight = float(row.get("sample_weight", 1.0))
            sample_weights.append(weight)
            is_pseudo.append(bool(row.get("is_pseudo", False)))
            
    if fit_le:
        label_encoder = LabelEncoder()
        labels_encoded = label_encoder.fit_transform(labels)
        return texts, labels_encoded, sample_weights, is_pseudo, label_encoder
    else:
        labels_encoded = label_encoder.transform(labels)
        return texts, labels_encoded, sample_weights, is_pseudo

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
    parser.add_argument("--data", required=True, help="JSONL with labels (true or mixed true/pseudo)")
    parser.add_argument("--out_dir", default="data/models/transformer/single_stage")
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--lr", type=float, default=2e-5)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    logger.info("Loading dataset from %s...", args.data)
    texts, labels, sample_weights, is_pseudo, le = load_data(args.data, fit_le=True)
    joblib.dump(le, out_dir / "label_encoder.joblib")
    
    n_total = len(texts)
    n_pseudo = sum(is_pseudo)
    n_true = n_total - n_pseudo
    logger.info("Total samples: %d (True: %d, Pseudo: %d)", n_total, n_true, n_pseudo)
    
    # Stratified validation split: ONLY from TRUE labels to ensure clean validation
    from sklearn.model_selection import train_test_split
    
    true_indices = [i for i, pseudo in enumerate(is_pseudo) if not pseudo]
    pseudo_indices = [i for i, pseudo in enumerate(is_pseudo) if pseudo]
    
    if len(true_indices) >= 20:
        true_train_idx, true_val_idx = train_test_split(
            true_indices,
            test_size=0.15,
            stratify=[labels[i] for i in true_indices],
            random_state=42,
        )
    else:
        # Fallback if no true distinction
        true_train_idx, true_val_idx = train_test_split(
            list(range(n_total)), test_size=0.15, stratify=labels, random_state=42
        )
        pseudo_indices = []

    # Final train set = true train + all pseudo
    train_indices = true_train_idx + pseudo_indices
    val_indices = true_val_idx
    
    logger.info("Train split: %d items (True: %d, Pseudo: %d) | Val split: %d items (100%% True)",
                len(train_indices), len(true_train_idx), len(pseudo_indices), len(val_indices))

    t_train_texts = [texts[i] for i in train_indices]
    t_train_labels = [labels[i] for i in train_indices]
    t_train_weights = [sample_weights[i] for i in train_indices]

    t_val_texts = [texts[i] for i in val_indices]
    t_val_labels = [labels[i] for i in val_indices]
    t_val_weights = [sample_weights[i] for i in val_indices]

    logger.info("Loading tokenizer...")
    tokenizer = AutoTokenizer.from_pretrained("sentence-transformers/all-MiniLM-L12-v2")
    tokenizer.save_pretrained(str(out_dir / "tokenizer"))

    logger.info("Encoding datasets...")
    t_train_enc = tokenizer(t_train_texts, truncation=True, padding="max_length", max_length=MAX_LENGTH)
    t_val_enc = tokenizer(t_val_texts, truncation=True, padding="max_length", max_length=MAX_LENGTH)

    t_train_ds = TorrentDataset(t_train_enc, t_train_labels, t_train_weights)
    t_val_ds = TorrentDataset(t_val_enc, t_val_labels, t_val_weights)
    t_val_dl = DataLoader(t_val_ds, batch_size=BATCH_SIZE)
    t_train_dl = DataLoader(t_train_ds, batch_size=BATCH_SIZE, shuffle=True)

    # Moderate Capped Class Weights (computed on train labels)
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
    if torch.cuda.is_available():
        device = torch.device("cuda")
    else:
        device = torch.device("cpu")
    logger.info("Using device: %s", device)
    model.to(device)
    class_weights_tensor = class_weights_tensor.to(device)

    logger.info("=== Phase: Transformer Training (%d Epochs, lr=%.1e) ===", args.epochs, args.lr)
    optimizer = AdamW(model.parameters(), lr=args.lr)
    epochs = args.epochs
    scheduler = get_linear_schedule_with_warmup(optimizer, num_warmup_steps=len(t_train_dl)//10, num_training_steps=len(t_train_dl)*epochs)
    
    best_f1 = 0
    for epoch in range(epochs):
        model.train()
        total_loss = 0
        for step, batch in enumerate(t_train_dl):
            optimizer.zero_grad()
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels_tensor = batch["labels"].to(device)
            s_weights = batch["sample_weights"].to(device)
            
            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            loss_fct = torch.nn.CrossEntropyLoss(weight=class_weights_tensor, reduction="none")
            per_sample_loss = loss_fct(outputs.logits.view(-1, len(le.classes_)), labels_tensor.view(-1))
            loss = (per_sample_loss * s_weights).sum() / (s_weights.sum() + 1e-8)
            
            loss.backward()
            optimizer.step()
            scheduler.step()
            total_loss += loss.item()
            
        acc, f1 = evaluate(model, t_val_dl, device, le)
        logger.info(f"Epoch {epoch+1}/{epochs} Validation: Acc={acc:.3f}, MacroF1={f1:.3f}")
        if f1 > best_f1:
            best_f1 = f1
            model.save_pretrained(out_dir / "model")
            logger.info("Saved best model (MacroF1=%.4f)", best_f1)

if __name__ == "__main__":
    main()
