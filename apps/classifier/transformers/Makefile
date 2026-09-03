.PHONY: train evaluate export-onnx export-labeled export pseudo-label test train-pseudo

train:
	python src/train_transformer.py \
		--data data/labeled_data/train.jsonl \
		--config config/modernbert.yaml \
		--out_dir data/models/modernbert_base

evaluate:
	python src/evaluate.py \
		--model_dir data/models/modernbert_base \
		--benchmark data/labeled_data/test.jsonl

export-onnx:
	python src/export_onnx.py --model_dir data/models/modernbert_base

export-labeled:
	python src/export_labeled.py \
		--output data/labeled_data/merged.jsonl \
		--split

export:
	python src/export_onnx.py --model_dir data/models/modernbert_base

pseudo-label:
	python src/pseudo_label.py \
		--count 20000 \
		--threshold 0.95 \
		--output data/labeled_data/pseudo_labels.jsonl

train-pseudo:
	cat data/labeled_data/train.jsonl data/labeled_data/pseudo_labels.jsonl > data/labeled_data/train_combined.jsonl
	python src/train_transformer.py \
		--data data/labeled_data/train_combined.jsonl \
		--config config/modernbert.yaml \
		--out_dir data/models/modernbert_pseudo_10k

test:
	python -m pytest tests/ -v
