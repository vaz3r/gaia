#!/usr/bin/env python3
"""Export trained DistilBERT to ONNX + int8 quantization with verification."""

import sys
import json
import logging
from pathlib import Path

logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='%(asctime)s %(levelname)s %(message)s')
logger = logging.getLogger('export_onnx')

import numpy as np
import torch
from transformers import AutoTokenizer, AutoModelForSequenceClassification
import joblib

sys.path.insert(0, '.')
from src.core.text_builder import build_input_text

MODEL_DIR = Path("data/models/transformer")
ONNX_PATH = MODEL_DIR / "model.onnx"
QUANT_PATH = MODEL_DIR / "model_int8.onnx"
MAX_LENGTH = 128
VERIFY_SAMPLES = 20


def main():
    # Load trained model + tokenizer
    logger.info("Loading trained model from %s...", MODEL_DIR / "model")
    tokenizer = AutoTokenizer.from_pretrained(str(MODEL_DIR / "tokenizer"))
    model = AutoModelForSequenceClassification.from_pretrained(str(MODEL_DIR / "model"))

    # CRITICAL: move to CPU before export (MPS causes export issues)
    model = model.cpu().eval()
    logger.info("Model loaded on CPU, %d params", sum(p.numel() for p in model.parameters()))

    # Dummy input for export
    dummy_text = "Name: Sample torrent\nFiles: 5  Size: 123456789\nTop dirs: file1.mkv, file2.mkv"
    dummy_enc = tokenizer(
        dummy_text,
        return_tensors="pt",
        truncation=True,
        padding="max_length",
        max_length=MAX_LENGTH,
    )
    input_ids = dummy_enc["input_ids"]
    attention_mask = dummy_enc["attention_mask"]

    logger.info("Exporting to ONNX (opset 14)...")
    MODEL_DIR.mkdir(parents=True, exist_ok=True)

    torch.onnx.export(
        model,
        (input_ids, attention_mask),
        str(ONNX_PATH),
        input_names=["input_ids", "attention_mask"],
        output_names=["logits"],
        dynamic_axes={
            "input_ids": {0: "batch_size"},
            "attention_mask": {0: "batch_size"},
            "logits": {0: "batch_size"},
        },
        opset_version=14,
        do_constant_folding=True,
    )
    logger.info("ONNX export: %s (%.1f MB)", ONNX_PATH, ONNX_PATH.stat().st_size / 1e6)

    # Int8 quantization
    logger.info("Quantizing to int8...")
    from onnxruntime.quantization import quantize_dynamic, QuantType

    quantize_dynamic(
        str(ONNX_PATH),
        str(QUANT_PATH),
        weight_type=QuantType.QInt8,
    )
    logger.info("Quantized: %s (%.1f MB)", QUANT_PATH, QUANT_PATH.stat().st_size / 1e6)

    # ── Verification ────────────────────────────────────────────────────────
    logger.info("Verifying ONNX vs PyTorch (sample predictions)...")
    import onnxruntime as ort

    ort_session = ort.InferenceSession(str(QUANT_PATH), providers=["CPUExecutionProvider"])

    # Load labeled data for test samples
    records = []
    with open("data/labeled.jsonl") as f:
        for line in f:
            if line.strip():
                records.append(json.loads(line))
    le = joblib.load(MODEL_DIR / "label_encoder.joblib")

    # Pick diverse samples
    indices = [0, 50, 100, 200, 300, 400, 500, 600, 700, 800,
               10, 60, 150, 250, 350, 450, 550, 650, 750, 850]
    indices = [i for i in indices if i < len(records)]

    match_count = 0
    total = 0

    for idx in indices:
        r = records[idx]
        text = build_input_text(r)
        true_label = r["label_category"]

        # PyTorch inference
        enc = tokenizer(text, return_tensors="pt", truncation=True, padding="max_length", max_length=MAX_LENGTH)
        with torch.no_grad():
            pt_logits = model(**enc).logits.numpy()[0]

        # ONNX inference
        ort_inputs = {
            "input_ids": enc["input_ids"].numpy().astype(np.int64),
            "attention_mask": enc["attention_mask"].numpy().astype(np.int64),
        }
        onnx_logits = ort_session.run(None, ort_inputs)[0][0]

        pt_pred = le.classes_[np.argmax(pt_logits)]
        onnx_pred = le.classes_[np.argmax(onnx_logits)]
        logits_close = np.allclose(pt_logits, onnx_logits, atol=1e-2)

        match = pt_pred == onnx_pred
        if match:
            match_count += 1
        total += 1

        status = "OK" if match and logits_close else "MISMATCH"
        if not match or not logits_close:
            logger.warning(
                "  [%d] true=%s pt=%s onnx=%s logits_close=%s -> %s",
                idx, true_label, pt_pred, onnx_pred, logits_close, status,
            )
        else:
            logger.info(
                "  [%d] true=%s pt=%s onnx=%s -> OK",
                idx, true_label, pt_pred, onnx_pred,
            )

    agreement = match_count / total * 100
    logger.info("Verification: %d/%d predictions match (%.1f%%)", match_count, total, agreement)

    if agreement < 90:
        logger.error("VERIFICATION FAILED: prediction agreement < 90%%. Check export.")
        sys.exit(1)

    logger.info("Export complete. Files:")
    logger.info("  %s (%.1f MB)", ONNX_PATH, ONNX_PATH.stat().st_size / 1e6)
    logger.info("  %s (%.1f MB)", QUANT_PATH, QUANT_PATH.stat().st_size / 1e6)


if __name__ == "__main__":
    main()
