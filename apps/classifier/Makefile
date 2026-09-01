.PHONY: build run test evaluate clean push-remote-dev rebuild train-transformer export-onnx deploy-transformer train-v2 export-v2 evaluate-v2 export-labeled

IMAGE := torrent-classifier:latest

build:
	DOCKER_BUILDKIT=1 docker build -t $(IMAGE) .

rebuild:
	DOCKER_BUILDKIT=1 docker build --no-cache -t $(IMAGE) .

run:
	docker compose run --rm classifier

test:
	python -m pytest tests/ -v

evaluate:
	python src/evaluate.py \
		--predictions data/predictions/classified_sample.jsonl \
		--labels data/labeled_validation.jsonl

train-transformer:
	python src/train_transformer.py

export-onnx:
	python src/export_onnx.py

deploy-transformer: build
	docker save $(IMAGE) | docker --context remote-dev load

clean:
	docker rmi $(IMAGE) 2>/dev/null || true
	rm -rf data/predictions/*

push-remote-dev: build
	docker save $(IMAGE) | docker --context remote-dev load

build-remote-dev:
	rsync -az --exclude='.git' --exclude='__pycache__' --exclude='*.pyc' \
		. core@100.99.147.104:~/classifier/
	ssh core@100.99.147.104 "cd ~/classifier && DOCKER_BUILDKIT=1 docker build -t $(IMAGE) ."

# ── v2 pipeline (Gemini-labeled, raw metadata) ────────────────────────────────

train-v2:
	python src/train_transformer.py \
		--data data/gemini_labeled/train.jsonl \
		--config config/transformer_v2.yaml \
		--out_dir data/models/transformer_v2

export-v2:
	python src/export_onnx.py

export-labeled:
	python src/export_labeled.py --split

evaluate-v2:
	python src/evaluate.py \
		--predictions data/predictions/v2.jsonl \
		--labels data/gemini_labeled/test.jsonl
