"""ONNX Runtime inference backend for DistilBERT torrent classifier."""

from __future__ import annotations

import logging
from pathlib import Path

import numpy as np

logger = logging.getLogger(__name__)


class TransformerOnnxBackend:
    """Batch inference using quantized ONNX DistilBERT model."""

    def __init__(
        self,
        model_path: str = "data/models/transformer/model_int8.onnx",
        tokenizer_path: str = "data/models/transformer/tokenizer",
        max_length: int = 128,
        num_threads: int = 4,
    ):
        import os
        import onnxruntime as ort
        from transformers import AutoTokenizer

        self.max_length = max_length

        # Limit ONNX Runtime threads to avoid affinity warnings in containers
        os.environ.setdefault("OMP_NUM_THREADS", str(num_threads))
        os.environ.setdefault("ONNX_NUM_THREADS", str(num_threads))

        sess_options = ort.SessionOptions()
        sess_options.intra_op_num_threads = num_threads
        sess_options.inter_op_num_threads = num_threads
        sess_options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL

        logger.info("Loading ONNX model from %s (threads=%d)...", model_path, num_threads)
        self.session = ort.InferenceSession(
            model_path,
            sess_options=sess_options,
            providers=["CPUExecutionProvider"],
        )
        logger.info("ONNX providers: %s", self.session.get_providers())

        logger.info("Loading tokenizer from %s...", tokenizer_path)
        self.tokenizer = AutoTokenizer.from_pretrained(tokenizer_path)

    def predict(self, texts: list[str], batch_size: int = 32) -> tuple[np.ndarray, np.ndarray]:
        """Classify a batch of texts.

        Returns:
            (probs, predictions) where probs is (N, num_classes) float32
            and predictions is (N,) int64 of class indices.
        """
        all_probs = []

        for i in range(0, len(texts), batch_size):
            batch = texts[i : i + batch_size]
            enc = self.tokenizer(
                batch,
                truncation=True,
                padding="max_length",
                max_length=self.max_length,
                return_tensors="np",
            )
            inputs = {
                "input_ids": enc["input_ids"].astype(np.int64),
                "attention_mask": enc["attention_mask"].astype(np.int64),
            }
            logits = self.session.run(None, inputs)[0]  # (batch, num_classes)

            # Softmax
            exp = np.exp(logits - logits.max(axis=-1, keepdims=True))
            probs = exp / exp.sum(axis=-1, keepdims=True)
            all_probs.append(probs)

        all_probs = np.concatenate(all_probs, axis=0)
        predictions = all_probs.argmax(axis=-1)
        return all_probs, predictions
