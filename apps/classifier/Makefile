.PHONY: help install serve train shard clean test

PYTHON ?= venv/bin/python
UVICORN ?= venv/bin/uvicorn

help:
	@echo "Gaia Torrent Classifier Commands:"
	@echo "  make serve                Start Web Studio API & UI (http://localhost:8000)"
	@echo "  make train                Retrain 10-class calibrated model on labeled data"
	@echo "  make shard START=00 END=3f Run high-throughput streaming classifier on infohash shard"
	@echo "  make install              Install dependencies in virtual environment"
	@echo "  make clean                Clean temporary cache files"

install:
	$(PYTHON) -m pip install -r requirements.txt -r requirements-dev.txt

serve:
	$(UVICORN) web.app:app --host 0.0.0.0 --port 8000 --reload

train:
	$(PYTHON) src/train.py

shard:
	$(PYTHON) scripts/stream_classifier.py --start-hex $(or $(START),00) --end-hex $(or $(END),ff)

clean:
	find . -type d -name "__pycache__" -exec rm -rf {} +
	find . -type d -name ".pytest_cache" -exec rm -rf {} +
	find . -type f -name "*.pyc" -delete

test:
	$(PYTHON) -m pytest tests/ -v
