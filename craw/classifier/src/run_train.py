#!/usr/bin/env python3
"""Combined training script with progress output."""
import sys
import json
import time
import logging
from pathlib import Path

logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='%(asctime)s %(levelname)s %(message)s')
logger = logging.getLogger('train')

import yaml
import numpy as np
import joblib
from sklearn.svm import LinearSVC
from sklearn.calibration import CalibratedClassifierCV
from sklearn.metrics import classification_report
from sklearn.model_selection import train_test_split
from sklearn.preprocessing import LabelEncoder

sys.path.insert(0, '.')
from src.core.text_builder import build_input_text

with open('config/embedding.yaml') as f:
    config = yaml.safe_load(f)

logger.info('Loading labels...')
records = []
with open('data/labeled.jsonl') as f:
    for line in f:
        if line.strip():
            records.append(json.loads(line))
logger.info('Loaded %d records', len(records))

texts = [build_input_text(r, config) for r in records]
labels = [r['label_category'] for r in records]

le = LabelEncoder()
y = le.fit_transform(labels)
logger.info('Classes: %s', list(le.classes_))

cache_path = Path('data/models/embeddings_cache.npy')
if cache_path.exists():
    logger.info('Loading cached embeddings from %s', cache_path)
    X = np.load(cache_path)
    logger.info('Loaded embeddings: shape=%s', X.shape)
else:
    from src.backends.embedding_backend import EmbeddingBackend
    logger.info('Loading embedding model...')
    t0 = time.time()
    backend = EmbeddingBackend(config['embedding']['model_name'], cache_dir=config['embedding'].get('cache_dir'))
    logger.info('Model loaded in %.1fs', time.time() - t0)

    logger.info('Embedding %d texts with batch_size=%d...', len(texts), config['embedding']['batch_size'])
    sys.stderr.flush()
    t0 = time.time()
    X = backend.embed(texts, batch_size=config['embedding']['batch_size'])
    elapsed = time.time() - t0
    logger.info('Embedded in %.1fs, shape=%s', elapsed, X.shape)
    logger.info('Per-torrent: %.1fms', elapsed / len(texts) * 1000)
    sys.stderr.flush()

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    np.save(str(cache_path), X)
    logger.info('Saved embeddings cache')

X_train, X_test, y_train, y_test = train_test_split(X, y, test_size=0.2, random_state=42, stratify=y)
logger.info('Split: train=%d test=%d', len(X_train), len(X_test))

base_clf = LinearSVC(max_iter=2000, C=1.0, class_weight='balanced', random_state=42)
clf = CalibratedClassifierCV(base_clf, cv=3)
clf.fit(X_train, y_train)

y_pred = clf.predict(X_test)
report = classification_report(y_test, y_pred, target_names=list(le.classes_), zero_division=0)
print(report)

out_dir = Path('data/models')
joblib.dump(clf, out_dir / 'logreg_category.joblib')
joblib.dump(le, out_dir / 'label_encoder.joblib')
with open(out_dir / 'classification_report.txt', 'w') as f:
    f.write(report)
logger.info('Done! Models saved.')
