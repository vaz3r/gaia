.PHONY: build run test evaluate clean push-remote-dev rebuild

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

clean:
	docker rmi $(IMAGE) 2>/dev/null || true
	rm -rf data/predictions/*

push-remote-dev: build
	docker save $(IMAGE) | docker --context remote-dev load

build-remote-dev:
	rsync -az --exclude='.git' --exclude='__pycache__' --exclude='*.pyc' \
		. core@100.99.147.104:~/classifier/
	ssh core@100.99.147.104 "cd ~/classifier && DOCKER_BUILDKIT=1 docker build -t $(IMAGE) ."
