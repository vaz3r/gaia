#!/usr/bin/env python3
import time
import sys
sys.path.insert(0, '.')
from src.backends.embedding_backend import EmbeddingBackend

t0 = time.time()
backend = EmbeddingBackend(cache_dir="/app/data/fastembed_cache")
print("Model loaded: %.1fs" % (time.time()-t0))

texts = ["Breaking Bad S01E01 720p", "Adobe Photoshop CC 2024", "The Legend of Zelda Tears of the Kingdom", "Drake - Views", "Planet Earth II 4K"]

t0 = time.time()
embs = backend.embed(texts, batch_size=32)
elapsed = time.time()-t0
print("5 texts: %.3fs (%.1fms/torrent)" % (elapsed, elapsed/len(texts)*1000))
print("Shape:", embs.shape)
