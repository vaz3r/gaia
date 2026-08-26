#!/bin/bash
source venv/bin/activate
echo "Training Iteration 10 on 4,567 true labels..."
PYTHONPATH=. python3 src/train_transformer.py --data data/training_combined_v10_true.jsonl
echo "Exporting Iteration 10 ONNX..."
python3 src/export_onnx.py
echo "Evaluating Iteration 10..."
PYTHONPATH=. python3 src/classify_batch.py --mode transformer --input data/manual_eval_set_1000.jsonl --output data/predictions_natural_test_set_iter10_1000.jsonl
PYTHONPATH=. python3 src/evaluate.py --predictions data/predictions_natural_test_set_iter10_1000.jsonl --labels data/manual_eval_set_1000.jsonl --report data/manual_eval_report_iter10_1000.txt
cat data/manual_eval_report_iter10_1000.txt
