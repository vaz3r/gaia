#!/bin/bash
source venv/bin/activate
echo "Training Iteration 9 on 4,061 true labels..."
PYTHONPATH=. python3 src/train_transformer.py --data data/training_combined_v9_true.jsonl
echo "Exporting Iteration 9 ONNX..."
python3 src/export_onnx.py
echo "Evaluating Iteration 9..."
PYTHONPATH=. python3 src/classify_batch.py --mode transformer --input data/manual_eval_set_1000.jsonl --output data/predictions_natural_test_set_iter9_1000.jsonl
PYTHONPATH=. python3 src/evaluate.py --predictions data/predictions_natural_test_set_iter9_1000.jsonl --labels data/manual_eval_set_1000.jsonl --report data/manual_eval_report_iter9_1000.txt
cat data/manual_eval_report_iter9_1000.txt
