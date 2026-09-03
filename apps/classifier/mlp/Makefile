.PHONY: train evaluate export-labeled export pseudo-label test

train:
	python src/train_mlp.py \
		--data data/labeled_data/train.jsonl \
		--config config/mlp.yaml \
		--out_dir data/models/mlp

evaluate:
	python src/evaluate.py \
		--model data/models/mlp/torrent_classifier.joblib \
		--benchmark data/labeled_data/test.jsonl

export-labeled:
	python src/export_labeled.py \
		--output data/labeled_data/merged.jsonl \
		--split

export:
	python src/export_model.py \
		--model data/models/mlp/torrent_classifier.joblib \
		--output data/models/mlp/torrent_classifier_export.joblib

pseudo-label:
	python src/pseudo_label.py \
		--count 20000 \
		--threshold 0.95 \
		--output data/labeled_data/pseudo_labels.jsonl

test:
	python -m pytest tests/ -v
