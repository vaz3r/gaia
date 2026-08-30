#!/usr/bin/env python3
"""
Baseline v2 Training and Direct Comparison with Baseline v1.
Trains sentence-transformers/all-MiniLM-L12-v2 on 3,953 Baseline v2 rows
(3,553 original + 400 reviewed targeted augmentation rows with sample_weight=0.70),
selects best checkpoint via validation macro F1, evaluates on the frozen 300-record
Gold Pilot v1 reference benchmark, and computes direct comparison deltas.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import random
import subprocess
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

import joblib
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
import torch.nn.functional as F
from torch.optim import AdamW
from torch.utils.data import DataLoader, Dataset
import transformers
from transformers import (
    AutoConfig,
    AutoModelForSequenceClassification,
    AutoTokenizer,
    get_linear_schedule_with_warmup,
)

# Core text builder import
apps_dir = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(apps_dir / "src"))
from core.text_builder import build_input_text

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("train_baseline_v2")

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

EXPECTED_CHECKSUMS = {
    "validation.jsonl": "d0942b0e9ccb193835b568ae3ae94ce2404e3ff5db9604ba097dc94eadd42791",
    "reference_eval_v1.jsonl": "16c45e847a4626a9ef468c1728bc1786949470a4cb2adb1ef12520ba4a6fb4f2",
}

SEED = 42
MAX_LENGTH = 256
BATCH_SIZE = 16
LEARNING_RATE = 2e-5
WEIGHT_DECAY = 0.01
EPOCHS = 4
WARMUP_RATIO = 0.10
MAX_GRAD_NORM = 1.0


def set_seed(seed: int = SEED):
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)


def calc_sha256(path: Path | str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


class TorrentDataset(Dataset):
    def __init__(self, encodings, labels, sample_weights):
        self.encodings = encodings
        self.labels = labels
        self.sample_weights = np.array(sample_weights, dtype=np.float32)

    def __getitem__(self, idx):
        item = {key: torch.tensor(val[idx]) for key, val in self.encodings.items()}
        item["labels"] = torch.tensor(self.labels[idx], dtype=torch.long)
        item["sample_weights"] = torch.tensor(self.sample_weights[idx], dtype=torch.float32)
        return item

    def __len__(self):
        return len(self.labels)


def verify_and_load_data(
    train_path: Path,
    val_path: Path,
    ref_path: Path,
    tokenizer: AutoTokenizer,
) -> tuple[dict, dict, dict]:
    val_sha = calc_sha256(val_path)
    ref_sha = calc_sha256(ref_path)

    if val_sha != EXPECTED_CHECKSUMS["validation.jsonl"]:
        raise ValueError(f"Validation checksum mismatch: {val_sha} != {EXPECTED_CHECKSUMS['validation.jsonl']}")
    if ref_sha != EXPECTED_CHECKSUMS["reference_eval_v1.jsonl"]:
        raise ValueError(f"Reference checksum mismatch: {ref_sha} != {EXPECTED_CHECKSUMS['reference_eval_v1.jsonl']}")

    def load_rows(path: Path) -> list[dict]:
        with open(path, "r", encoding="utf-8") as f:
            return [json.loads(line) for line in f if line.strip()]

    train_rows = load_rows(train_path)
    val_rows = load_rows(val_path)
    ref_rows = load_rows(ref_path)

    if len(train_rows) != 3953:
        raise ValueError(f"Expected 3953 train rows, got {len(train_rows)}")
    if len(val_rows) != 627:
        raise ValueError(f"Expected 627 validation rows, got {len(val_rows)}")
    if len(ref_rows) != 300:
        raise ValueError(f"Expected 300 reference rows, got {len(ref_rows)}")

    if any(r.get("is_pseudo") for r in val_rows):
        raise ValueError("Validation set contains pseudo-labeled rows!")

    for r in train_rows + val_rows + ref_rows:
        cat = r.get("label_category")
        if cat not in LABEL2ID:
            raise ValueError(f"Invalid label category '{cat}'")

    logger.info("Input verification passed successfully.")

    def process_data(rows: list[dict]):
        texts = [build_input_text(r) for r in rows]
        labels = [LABEL2ID[r["label_category"]] for r in rows]
        weights = [float(r.get("sample_weight", 1.0)) for r in rows]
        encodings = tokenizer(
            texts,
            truncation=True,
            padding=True,
            max_length=MAX_LENGTH,
            return_tensors=None,
        )
        dataset = TorrentDataset(encodings, labels, weights)
        return {
            "rows": rows,
            "texts": texts,
            "labels": labels,
            "weights": weights,
            "dataset": dataset,
        }

    train_data = process_data(train_rows)
    val_data = process_data(val_rows)
    ref_data = process_data(ref_rows)

    return train_data, val_data, ref_data


def compute_metrics(
    y_true: list[int],
    y_pred: list[int],
    class_names: list[str] = FIXED_CLASSES,
) -> dict:
    acc = accuracy_score(y_true, y_pred)
    macro_p, macro_r, macro_f1, _ = precision_recall_fscore_support(
        y_true, y_pred, labels=list(range(len(class_names))), average="macro", zero_division=0
    )
    w_p, w_r, w_f1, _ = precision_recall_fscore_support(
        y_true, y_pred, labels=list(range(len(class_names))), average="weighted", zero_division=0
    )

    p_per, r_per, f1_per, sup_per = precision_recall_fscore_support(
        y_true, y_pred, labels=list(range(len(class_names))), average=None, zero_division=0
    )

    per_class = {}
    for i, c in enumerate(class_names):
        per_class[c] = {
            "precision": float(p_per[i]),
            "recall": float(r_per[i]),
            "f1": float(f1_per[i]),
            "support": int(sup_per[i]),
        }

    cm = confusion_matrix(y_true, y_pred, labels=list(range(len(class_names))))

    return {
        "accuracy": float(acc),
        "macro_precision": float(macro_p),
        "macro_recall": float(macro_r),
        "macro_f1": float(macro_f1),
        "weighted_precision": float(w_p),
        "weighted_recall": float(w_r),
        "weighted_f1": float(w_f1),
        "per_class": per_class,
        "confusion_matrix": cm.tolist(),
    }


def evaluate(
    model: torch.nn.Module,
    dataloader: DataLoader,
    device: torch.device,
    class_weights_tensor: torch.Tensor,
) -> tuple[float, list[int], list[float], list[list[dict]]]:
    model.eval()
    total_loss = 0.0
    total_weight = 0.0

    all_preds = []
    all_confs = []
    all_top3 = []

    with torch.no_grad():
        for batch in dataloader:
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels = batch["labels"].to(device)
            sample_weights = batch["sample_weights"].to(device)

            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            logits = outputs.logits

            unreduced_ce = F.cross_entropy(
                logits, labels, weight=class_weights_tensor, reduction="none"
            )
            loss = (sample_weights * unreduced_ce).sum()
            total_loss += loss.item()
            total_weight += sample_weights.sum().item()

            probs = F.softmax(logits, dim=-1)
            confs, preds = torch.max(probs, dim=-1)

            for i in range(len(labels)):
                p_row = probs[i].cpu().numpy()
                pred_idx = int(preds[i].item())
                conf_val = float(confs[i].item())

                top3_indices = np.argsort(p_row)[::-1][:3]
                top3_list = [
                    {"label": ID2LABEL[int(idx)], "probability": float(p_row[idx])}
                    for idx in top3_indices
                ]

                all_preds.append(pred_idx)
                all_confs.append(conf_val)
                all_top3.append(top3_list)

    avg_loss = total_loss / (total_weight + 1e-8)
    return avg_loss, all_preds, all_confs, all_top3


def train_and_evaluate(
    train_path: str = "apps/classifier/data/baseline_v2/train.jsonl",
    val_path: str = "apps/classifier/data/baseline_v2/validation.jsonl",
    ref_path: str = "apps/classifier/data/gold_pilot_v1/reference_eval_v1.jsonl",
    out_dir: str = "apps/classifier/data/models/transformer/baseline_v2",
    base_model_name: str = "sentence-transformers/all-MiniLM-L12-v2",
) -> dict:
    set_seed(SEED)

    out_p = Path(out_dir)
    out_p.mkdir(parents=True, exist_ok=True)

    text_builder_path = Path("apps/classifier/src/core/text_builder.py")
    text_builder_sha = calc_sha256(text_builder_path) if text_builder_path.exists() else ""

    if torch.cuda.is_available():
        device = torch.device("cuda")
        device_name = torch.cuda.get_device_name(0)
    elif torch.backends.mps.is_available():
        device = torch.device("mps")
        device_name = "Apple Silicon MPS"
    else:
        device = torch.device("cpu")
        device_name = "CPU"

    logger.info(f"Using device: {device} ({device_name})")

    tokenizer = AutoTokenizer.from_pretrained(base_model_name)

    train_data, val_data, ref_data = verify_and_load_data(
        Path(train_path), Path(val_path), Path(ref_path), tokenizer
    )

    # Recompute balanced class weights on 3,953 rows, clipped to [0.5, 3.0]
    raw_class_weights = compute_class_weight(
        class_weight="balanced",
        classes=np.arange(len(FIXED_CLASSES)),
        y=np.array(train_data["labels"]),
    )
    clipped_class_weights = np.clip(raw_class_weights, 0.5, 3.0).astype(np.float32)
    class_weights_dict = {FIXED_CLASSES[i]: float(clipped_class_weights[i]) for i in range(8)}
    logger.info(f"Recomputed clipped class weights: {class_weights_dict}")

    class_weights_tensor = torch.tensor(clipped_class_weights, dtype=torch.float32).to(device)

    train_generator = torch.Generator()
    train_generator.manual_seed(SEED)

    train_loader = DataLoader(
        train_data["dataset"],
        batch_size=BATCH_SIZE,
        shuffle=True,
        generator=train_generator,
    )
    val_loader = DataLoader(
        val_data["dataset"],
        batch_size=BATCH_SIZE,
        shuffle=False,
    )
    ref_loader = DataLoader(
        ref_data["dataset"],
        batch_size=BATCH_SIZE,
        shuffle=False,
    )

    config = AutoConfig.from_pretrained(
        base_model_name,
        num_labels=8,
        id2label=ID2LABEL,
        label2id=LABEL2ID,
    )
    model = AutoModelForSequenceClassification.from_pretrained(
        base_model_name,
        config=config,
    )
    model.to(device)

    total_params = sum(p.numel() for p in model.parameters())
    trainable_params = sum(p.numel() for p in model.parameters() if p.requires_grad)

    model_arch = {
        "hidden_size": config.hidden_size,
        "num_hidden_layers": config.num_hidden_layers,
        "num_attention_heads": config.num_attention_heads,
        "classifier_dropout": getattr(config, "classifier_dropout", None),
        "hidden_dropout_prob": getattr(config, "hidden_dropout_prob", None),
        "attention_probs_dropout_prob": getattr(config, "attention_probs_dropout_prob", None),
        "total_parameters": total_params,
        "trainable_parameters": trainable_params,
    }

    optimizer = AdamW(model.parameters(), lr=LEARNING_RATE, weight_decay=WEIGHT_DECAY)
    total_steps = len(train_loader) * EPOCHS
    warmup_steps = int(WARMUP_RATIO * total_steps)
    scheduler = get_linear_schedule_with_warmup(
        optimizer,
        num_warmup_steps=warmup_steps,
        num_training_steps=total_steps,
    )

    logger.info(f"Starting training: {EPOCHS} epochs, {total_steps} steps, {warmup_steps} warmup steps.")

    best_epoch = -1
    best_val_f1 = -1.0
    best_val_loss = float("inf")
    best_model_state = None
    best_val_preds = None
    best_val_confs = None
    best_val_top3 = None
    best_val_metrics = None

    training_history = []

    for epoch in range(1, EPOCHS + 1):
        model.train()
        total_train_loss = 0.0
        total_train_weight = 0.0

        for batch_idx, batch in enumerate(train_loader):
            optimizer.zero_grad()
            input_ids = batch["input_ids"].to(device)
            attention_mask = batch["attention_mask"].to(device)
            labels = batch["labels"].to(device)
            sample_weights = batch["sample_weights"].to(device)

            outputs = model(input_ids=input_ids, attention_mask=attention_mask)
            logits = outputs.logits

            unreduced_ce = F.cross_entropy(
                logits, labels, weight=class_weights_tensor, reduction="none"
            )
            loss = (sample_weights * unreduced_ce).sum() / (sample_weights.sum() + 1e-8)

            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=MAX_GRAD_NORM)

            optimizer.step()
            scheduler.step()

            total_train_loss += (sample_weights * unreduced_ce).sum().item()
            total_train_weight += sample_weights.sum().item()

        avg_train_loss = total_train_loss / (total_train_weight + 1e-8)

        val_loss, val_preds, val_confs, val_top3 = evaluate(
            model, val_loader, device, class_weights_tensor
        )
        val_metrics = compute_metrics(val_data["labels"], val_preds)

        current_lr = scheduler.get_last_lr()[0]

        epoch_record = {
            "epoch": epoch,
            "train_loss": float(avg_train_loss),
            "val_loss": float(val_loss),
            "val_accuracy": float(val_metrics["accuracy"]),
            "val_macro_f1": float(val_metrics["macro_f1"]),
            "val_weighted_f1": float(val_metrics["weighted_f1"]),
            "learning_rate": float(current_lr),
        }
        training_history.append(epoch_record)

        logger.info(
            f"Epoch {epoch}/{EPOCHS} | Train Loss: {avg_train_loss:.4f} | "
            f"Val Loss: {val_loss:.4f} | Val Acc: {val_metrics['accuracy']:.4f} | "
            f"Val Macro F1: {val_metrics['macro_f1']:.4f} | Val Weighted F1: {val_metrics['weighted_f1']:.4f}"
        )

        is_best = False
        if val_metrics["macro_f1"] > best_val_f1 + 1e-6:
            is_best = True
        elif abs(val_metrics["macro_f1"] - best_val_f1) <= 1e-6:
            if val_loss < best_val_loss:
                is_best = True

        if is_best:
            best_val_f1 = val_metrics["macro_f1"]
            best_val_loss = val_loss
            best_epoch = epoch
            best_model_state = {k: v.cpu().clone() for k, v in model.state_dict().items()}
            best_val_preds = val_preds
            best_val_confs = val_confs
            best_val_top3 = val_top3
            best_val_metrics = val_metrics

    logger.info(f"Selected best epoch: {best_epoch} with Val Macro F1: {best_val_f1:.4f}")

    model.load_state_dict({k: v.to(device) for k, v in best_model_state.items()})

    model_dir = out_p / "model"
    tok_dir = out_p / "tokenizer"
    model.save_pretrained(model_dir)
    tokenizer.save_pretrained(tok_dir)

    le = LabelEncoder()
    le.classes_ = np.array(FIXED_CLASSES)
    joblib.dump(le, out_p / "label_encoder.joblib")

    with open(out_p / "training_history.json", "w", encoding="utf-8") as f:
        json.dump(training_history, f, indent=2)

    val_pred_file = out_p / "validation_predictions.jsonl"
    with open(val_pred_file, "w", encoding="utf-8") as f:
        for i, row in enumerate(val_data["rows"]):
            pred_item = {
                "infohash": row["infohash"],
                "true_label": row["label_category"],
                "predicted_label": ID2LABEL[best_val_preds[i]],
                "confidence": float(best_val_confs[i]),
                "top_3_candidates": best_val_top3[i],
            }
            f.write(json.dumps(pred_item, ensure_ascii=False) + "\n")

    ref_loss, ref_preds, ref_confs, ref_top3 = evaluate(
        model, ref_loader, device, class_weights_tensor
    )
    ref_pred_file = out_p / "reference_predictions.jsonl"
    with open(ref_pred_file, "w", encoding="utf-8") as f:
        for i, row in enumerate(ref_data["rows"]):
            pred_item = {
                "pilot_id": row["pilot_id"],
                "metadata_mode": row["metadata_mode"],
                "true_label": row["label_category"],
                "predicted_label": ID2LABEL[ref_preds[i]],
                "confidence": float(ref_confs[i]),
                "top_3_candidates": ref_top3[i],
                "label_resolution": row["label_resolution"],
                "reference_confidence": row["reference_confidence"],
            }
            f.write(json.dumps(pred_item, ensure_ascii=False) + "\n")

    ref_labels_all = ref_data["labels"]
    ref_metrics_all = compute_metrics(ref_labels_all, ref_preds)

    sparse_indices = [
        i for i, r in enumerate(ref_data["rows"]) if r["metadata_mode"] == "sparse_single_file"
    ]
    sparse_true = [ref_labels_all[i] for i in sparse_indices]
    sparse_pred = [ref_preds[i] for i in sparse_indices]
    sparse_metrics = compute_metrics(sparse_true, sparse_pred)

    rich_indices = [
        i for i, r in enumerate(ref_data["rows"]) if r["metadata_mode"] == "rich_multi_file"
    ]
    rich_true = [ref_labels_all[i] for i in rich_indices]
    rich_pred = [ref_preds[i] for i in rich_indices]
    rich_metrics = compute_metrics(rich_true, rich_pred)

    all_eval_metrics = {
        "validation_metrics": {
            "description": "Evaluation on frozen 627-row heuristic-labeled grouped validation split.",
            "metrics": best_val_metrics,
        },
        "reference_full_metrics": {
            "description": "Evaluation on frozen 300-row AI-adjudicated reference benchmark.",
            "support_total": len(ref_labels_all),
            "metrics": ref_metrics_all,
        },
        "reference_sparse_metrics": {
            "description": "Evaluation on 200-row sparse single-file natural stratum reference subset.",
            "support_total": len(sparse_indices),
            "metrics": sparse_metrics,
        },
        "reference_rich_metrics": {
            "description": "Evaluation on 100-row rich multi-file diagnostic stratum reference subset.",
            "support_total": len(rich_indices),
            "metrics": rich_metrics,
        },
    }

    with open(out_p / "evaluation_metrics.json", "w", encoding="utf-8") as f:
        json.dump(all_eval_metrics, f, indent=2)

    git_commit = ""
    git_dirty = False
    try:
        git_commit = subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip()
        git_dirty = bool(subprocess.check_output(["git", "status", "--porcelain"]).strip())
    except Exception:
        pass

    manifest = {
        "run_name": "baseline-v2-minilm-l12-fp32",
        "creation_timestamp": datetime.now(timezone.utc).isoformat(),
        "git_commit": git_commit,
        "dirty_working_tree": git_dirty,
        "source_file_paths_and_checksums": {
            "train.jsonl": {"path": train_path, "sha256": calc_sha256(train_path)},
            "validation.jsonl": {"path": val_path, "sha256": calc_sha256(val_path)},
            "reference_eval_v1.jsonl": {"path": ref_path, "sha256": calc_sha256(ref_path)},
        },
        "text_builder_checksum": text_builder_sha,
        "base_model_identifier": base_model_name,
        "resolved_model_architecture": model_arch,
        "fixed_label_mapping": {
            "id2label": ID2LABEL,
            "label2id": LABEL2ID,
        },
        "training_configuration": {
            "random_seed": SEED,
            "epochs": EPOCHS,
            "batch_size": BATCH_SIZE,
            "learning_rate": LEARNING_RATE,
            "optimizer": "AdamW",
            "weight_decay": WEIGHT_DECAY,
            "scheduler": "linear_decay_with_warmup",
            "warmup_ratio": WARMUP_RATIO,
            "gradient_clipping_max_norm": MAX_GRAD_NORM,
            "max_sequence_length": MAX_LENGTH,
            "precision": "FP32",
            "checkpoint_selection_metric": "validation_macro_f1",
        },
        "calculated_class_weights": class_weights_dict,
        "device": str(device),
        "device_description": device_name,
        "python_version": sys.version,
        "pytorch_version": torch.__version__,
        "transformers_version": transformers.__version__,
        "sklearn_version": sklearn.__version__,
        "total_optimizer_steps": total_steps,
        "warmup_steps": warmup_steps,
        "selected_epoch": best_epoch,
        "checkpoint_selection_metric": "validation_macro_f1",
        "best_validation_metrics": {
            "accuracy": best_val_metrics["accuracy"],
            "macro_f1": best_val_metrics["macro_f1"],
            "weighted_f1": best_val_metrics["weighted_f1"],
        },
        "reference_metrics": {
            "full_accuracy": ref_metrics_all["accuracy"],
            "full_macro_f1": ref_metrics_all["macro_f1"],
            "full_weighted_f1": ref_metrics_all["weighted_f1"],
            "sparse_accuracy": sparse_metrics["accuracy"],
            "sparse_macro_f1": sparse_metrics["macro_f1"],
            "rich_accuracy": rich_metrics["accuracy"],
            "rich_macro_f1": rich_metrics["macro_f1"],
        },
        "output_artifact_checksums": {
            "validation_predictions.jsonl": calc_sha256(val_pred_file),
            "reference_predictions.jsonl": calc_sha256(ref_pred_file),
            "evaluation_metrics.json": calc_sha256(out_p / "evaluation_metrics.json"),
            "training_history.json": calc_sha256(out_p / "training_history.json"),
            "label_encoder.joblib": calc_sha256(out_p / "label_encoder.joblib"),
        },
        "known_limitations": [
            "training labels include 400 targeted augmentation examples with sample_weight=0.70",
            "the reference set is same-model dual-pass AI-reviewed and same-model adjudicated",
            "the reference set has only four Documentary records",
            "confidence values represent raw softmax probabilities and are uncalibrated",
        ],
    }

    with open(out_p / "run_manifest.json", "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, ensure_ascii=False)

    logger.info("Run manifest and all artifacts saved successfully.")
    return {
        "manifest": manifest,
        "best_val_metrics": best_val_metrics,
        "ref_metrics_all": ref_metrics_all,
        "sparse_metrics": sparse_metrics,
        "rich_metrics": rich_metrics,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Train and evaluate FP32 MiniLM Baseline v2.")
    parser.add_argument("--train", default="apps/classifier/data/baseline_v2/train.jsonl")
    parser.add_argument("--val", default="apps/classifier/data/baseline_v2/validation.jsonl")
    parser.add_argument("--ref", default="apps/classifier/data/gold_pilot_v1/reference_eval_v1.jsonl")
    parser.add_argument("--out_dir", default="apps/classifier/data/models/transformer/baseline_v2")
    args = parser.parse_args()

    train_and_evaluate(
        train_path=args.train,
        val_path=args.val,
        ref_path=args.ref,
        out_dir=args.out_dir,
    )
