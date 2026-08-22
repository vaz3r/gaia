from __future__ import annotations

import json
import logging
import numpy as np

from .embedding_backend import EmbeddingBackend

logger = logging.getLogger(__name__)


class AnchorClassifier:
    def __init__(self, backend: EmbeddingBackend, anchors_path: str = "data/anchors.json"):
        self.backend = backend

        with open(anchors_path, encoding="utf-8") as f:
            anchor_data = json.load(f)

        self.categories = []
        self.anchor_texts = []
        for entry in anchor_data:
            cat = entry["category"]
            for anchor in entry["anchors"]:
                self.categories.append(cat)
                self.anchor_texts.append(anchor)

        logger.info("Embedding %d anchors across %d categories",
                     len(self.anchor_texts), len(set(self.categories)))
        self.anchor_embeddings = backend.embed(self.anchor_texts)
        self.categories = np.array(self.categories)

    def classify(self, text: str) -> tuple[str, float]:
        vec = self.backend.embed_single(text)
        sims = np.dot(self.anchor_embeddings, vec)

        best_idx = int(np.argmax(sims))
        best_sim = float(sims[best_idx])
        best_cat = self.categories[best_idx]

        return best_cat, round(best_sim, 4)
