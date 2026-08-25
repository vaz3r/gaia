#!/bin/bash
set -e

echo "Training Stage 1 (Binary Filter)..."
python3 src/train_transformer.py --data data/labeled.jsonl --stage 1

echo "Exporting Stage 1..."
export PYTHONPATH=.
python3 src/export_onnx.py --stage 1

echo "Stage 1 complete!"
