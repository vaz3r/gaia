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

COPY src/ ./src/
COPY prompts/ ./prompts/
COPY config/ ./config/
COPY data/anchors.json ./data/anchors.json

# Copy trained transformer model (ONNX int8 + tokenizer + label encoder only)
COPY data/models/transformer/model_int8.onnx ./data/models/transformer/model_int8.onnx
COPY data/models/transformer/tokenizer/ ./data/models/transformer/tokenizer/
COPY data/models/transformer/label_encoder.joblib ./data/models/transformer/label_encoder.joblib

RUN mkdir -p /app/data/predictions /app/data/models

RUN useradd -m classifier && chown -R classifier:classifier /app
USER classifier

ENV FASTEMBED_CACHE_DIR=/app/data/fastembed_cache
ENV HF_HOME=/app/data/hf_cache
ENV PYTHONPATH=/app

ENTRYPOINT ["python", "src/classify_batch.py"]
