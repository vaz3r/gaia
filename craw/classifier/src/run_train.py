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
from sklearn.ensemble import GradientBoostingClassifier, RandomForestClassifier, VotingClassifier
from sklearn.svm import LinearSVC
from sklearn.calibration import CalibratedClassifierCV
from sklearn.metrics import classification_report, confusion_matrix
from sklearn.model_selection import StratifiedKFold, cross_val_score, train_test_split
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

logger.info('Computing anchor similarity features...')
from src.backends.embedding_backend import EmbeddingBackend
backend = EmbeddingBackend(config['embedding']['model_name'], cache_dir=config['embedding'].get('cache_dir'))
anchor_file = config.get('classifier', {}).get('anchor_file', 'data/anchors.json')
with open(anchor_file) as f:
    anchor_data = json.load(f)
anchor_texts = []
anchor_cats = []
for entry in anchor_data:
    for anchor in entry['anchors']:
        anchor_texts.append(anchor)
        anchor_cats.append(entry['category'])
anchor_embs = backend.embed(anchor_texts)
anchor_cats = np.array(anchor_cats)

cat_names = list(le.classes_)
n_anchors = len(anchor_texts)
n_cats = len(cat_names)
cat_idx = {c: i for i, c in enumerate(cat_names)}

def make_anchor_features(embs):
    sims = embs @ anchor_embs.T
    max_per_cat = np.zeros((len(embs), n_cats))
    for ci, cn in enumerate(cat_names):
        mask = anchor_cats == cn
        if mask.any():
            max_per_cat[:, ci] = sims[:, mask].max(axis=1)
    mean_per_cat = np.zeros((len(embs), n_cats))
    for ci, cn in enumerate(cat_names):
        mask = anchor_cats == cn
        if mask.any():
            mean_per_cat[:, ci] = sims[:, mask].mean(axis=1)
    return np.hstack([max_per_cat, mean_per_cat, sims.max(axis=1, keepdims=True), sims.mean(axis=1, keepdims=True)])

X_train_aug = np.hstack([X_train, make_anchor_features(X_train)])
X_test_aug = np.hstack([X_test, make_anchor_features(X_test)])
logger.info('Augmented features: %d -> %d (added %d anchor features)', X.shape[1], X_train_aug.shape[1], X_train_aug.shape[1] - X.shape[1])

candidates = {
    'rf': RandomForestClassifier(n_estimators=300, max_depth=None, class_weight='balanced', random_state=42, n_jobs=-1),
    'svm': CalibratedClassifierCV(LinearSVC(max_iter=5000, C=1.0, class_weight='balanced', random_state=42), cv=3),
}

best_name = None
best_score = 0
best_clf = None
for name, clf in candidates.items():
    logger.info('Training %s...', name)
    t0 = time.time()
    clf.fit(X_train_aug, y_train)
    train_time = time.time() - t0
    y_pred = clf.predict(X_test_aug)
    acc = (y_pred == y_test).mean()
    report = classification_report(y_test, y_pred, target_names=cat_names, zero_division=0, output_dict=True)
    macro_f1 = report['macro avg']['f1-score']
    logger.info('%s: accuracy=%.3f macro_f1=%.3f time=%.1fs', name, acc, macro_f1, train_time)
    if macro_f1 > best_score:
        best_score = macro_f1
        best_name = name
        best_clf = clf

logger.info('Best: %s (macro_f1=%.3f)', best_name, best_score)

y_pred = best_clf.predict(X_test_aug)
report = classification_report(y_test, y_pred, target_names=cat_names, zero_division=0)
print(report)
print('\nConfusion matrix:')
print(confusion_matrix(y_test, y_pred))

out_dir = Path('data/models')
joblib.dump(best_clf, out_dir / 'logreg_category.joblib')
joblib.dump(le, out_dir / 'label_encoder.joblib')
with open(out_dir / 'classification_report.txt', 'w') as f:
    f.write(report)
logger.info('Done! Saved best model: %s', best_name)
