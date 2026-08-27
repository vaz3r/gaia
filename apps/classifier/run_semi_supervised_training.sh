#!/bin/bash
set -e
source venv/bin/activate

echo "================================================================="
echo "Phase 3: Semi-Supervised Transformer Fine-Tuning"
echo "Dataset: data/training_semi_supervised_v1.jsonl (14,896 items)"
echo "True labels: 5,408 (weight=1.0) | Pseudo-labels: 9,488 (weight=0.4)"
echo "================================================================="

PYTHONPATH=. python3 src/train_transformer.py \
    --data data/training_semi_supervised_v1.jsonl \
    --out_dir data/models/transformer/single_stage \
    --epochs 3 \
    --lr 2e-5

echo "================================================================="
echo "Exporting INT8 Quantized ONNX model..."
echo "================================================================="
python3 src/export_onnx.py

echo "================================================================="
echo "Evaluating on 1,000-sample natural test set (NEVER touched)..."
echo "================================================================="
PYTHONPATH=. python3 src/classify_batch.py \
    --mode transformer \
    --input data/manual_eval_set_1000.jsonl \
    --output data/predictions_semi_supervised_v1_1000.jsonl

PYTHONPATH=. python3 src/evaluate.py \
    --predictions data/predictions_semi_supervised_v1_1000.jsonl \
    --labels data/manual_eval_set_1000.jsonl \
    --report data/manual_eval_report_semi_supervised_v1.txt

cat data/manual_eval_report_semi_supervised_v1.txt
