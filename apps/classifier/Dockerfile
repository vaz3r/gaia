# syntax=docker/dockerfile:1.4

ARG BASE_IMAGE=python:3.11-slim

FROM ${BASE_IMAGE}

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

COPY requirements.txt .

RUN --mount=type=cache,target=/root/.cache/pip \
    pip install --no-cache-dir -r requirements.txt

# Copy transformers approach
COPY transformers/src/ ./transformers/src/
COPY transformers/config/ ./transformers/config/
COPY transformers/data/models/transformer/model_int8.onnx ./transformers/data/models/transformer/model_int8.onnx
COPY transformers/data/models/transformer/tokenizer/ ./transformers/data/models/transformer/tokenizer/
COPY transformers/data/models/transformer/label_encoder.joblib ./transformers/data/models/transformer/label_encoder.joblib

# Copy shared files
COPY prompts/ ./prompts/
COPY data/anchors.json ./data/anchors.json

RUN mkdir -p /app/data/predictions /app/data/models

RUN useradd -m classifier && chown -R classifier:classifier /app
USER classifier

ENV FASTEMBED_CACHE_DIR=/app/data/fastembed_cache
ENV HF_HOME=/app/data/hf_cache
ENV PYTHONPATH=/app

ENTRYPOINT ["python", "transformers/src/classify_batch.py"]
