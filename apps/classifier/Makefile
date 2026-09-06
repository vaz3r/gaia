.PHONY: help install serve train retrain worker worker-dry-run shard rollback clean test

PYTHON ?= venv/bin/python
UVICORN ?= venv/bin/uvicorn

help:
	@echo "Gaia Torrent Classifier Commands:"
	@echo "  make serve                Start Web Studio API & UI (http://localhost:8000)"
	@echo "  make retrain              Trigger direct DB retraining with comparative quality gate"
	@echo "  make worker               Run continuous queue worker daemon"
	@echo "  make worker-dry-run       Test queue worker in safe dry-run mode"
	@echo "  make rollback VERSION=v2  Rollback active model to a previous version"
	@echo "  make shard START=00 END=3f Run batch streaming classifier on infohash shard"
	@echo "  make install              Install dependencies in virtual environment"
	@echo "  make clean                Clean temporary cache files"

install:
	$(PYTHON) -m pip install -r requirements.txt -r requirements-dev.txt

serve:
	$(UVICORN) web.app:app --host 0.0.0.0 --port 8000 --reload

retrain:
	$(PYTHON) scripts/retrain.py

worker:
	$(PYTHON) scripts/worker.py

worker-dry-run:
	$(PYTHON) scripts/worker.py --dry-run

rollback:
	$(PYTHON) -m src.model_manager rollback --version $(VERSION)

shard:
	$(PYTHON) scripts/stream_classifier.py --start-hex $(or $(START),00) --end-hex $(or $(END),ff)

clean:
	find . -type d -name "__pycache__" -exec rm -rf {} +
	find . -type d -name ".pytest_cache" -exec rm -rf {} +
	find . -type f -name "*.pyc" -delete

test:
	$(PYTHON) -m pytest tests/ -v
