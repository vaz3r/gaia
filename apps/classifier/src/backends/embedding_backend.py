from __future__ import annotations

import logging
import numpy as np

logger = logging.getLogger(__name__)


class EmbeddingBackend:
    def __init__(self, model_name: str = "BAAI/bge-small-en-v1.5", cache_dir: str | None = None, threads: int = 4, **kwargs):
        from fastembed import TextEmbedding

        logger.info("Loading FastEmbed model: %s", model_name)
        embed_kwargs = {"threads": threads}
        if cache_dir:
            embed_kwargs["cache_dir"] = cache_dir
        embed_kwargs.update(kwargs)
        self.model = TextEmbedding(model_name=model_name, **embed_kwargs)
        self.model_name = model_name
        self.dimension = self.model.embedding_size
        logger.info("FastEmbed model loaded: dim=%d", self.dimension)

    def embed(self, texts: list[str], batch_size: int = 64) -> np.ndarray:
        embeddings = list(self.model.embed(texts, batch_size=batch_size))
        return np.vstack(embeddings).astype(np.float32)

    def embed_single(self, text: str) -> np.ndarray:
        return self.embed([text])[0]
