import json
import time
from pathlib import Path
import numpy as np
import joblib
from sklearn.linear_model import SGDClassifier
from sklearn.preprocessing import LabelEncoder

import sys
sys.path.append(str(Path(__file__).parent))
from feature_extractor import TorrentFeatureExtractor

DATA_PATH = Path(__file__).parent.parent / "data" / "labeled_dataset.jsonl"
MODEL_OUTPUT_PATH = Path(__file__).parent.parent / "models" / "torrent_classifier_v2.joblib"

def main():
    print(f"Loading high-confidence training data from {DATA_PATH} (excluding Other)...", flush=True)
    with open(DATA_PATH, "r", encoding="utf-8") as f:
        records = [
            json.loads(line) for line in f 
            if json.loads(line).get('confidence') == 'high' 
            and json.loads(line).get('label_category') != 'Other'
        ]
        
    print(f"Total training records across 10 classes: {len(records):,}", flush=True)
    
    le = LabelEncoder()
    y = le.fit_transform([r['label_category'] for r in records])
    classes = list(le.classes_)
    print(f"Target classes ({len(classes)}): {classes}", flush=True)
    
    print("\nFitting feature extractor on all records...", flush=True)
    t0 = time.time()
    extractor = TorrentFeatureExtractor(max_features=250000)
    X = extractor.fit_transform(records)
    print(f"Features extracted: shape={X.shape} in {time.time() - t0:.1f}s", flush=True)
    
    print("\nTraining calibrated classifier (modified_huber)...", flush=True)
    t1 = time.time()
    clf = SGDClassifier(
        loss='modified_huber',
        penalty='l2',
        alpha=3e-5,
        max_iter=1000,
        class_weight='balanced',
        random_state=42
    )
    clf.fit(X, y)
    print(f"Classifier trained in {time.time() - t1:.1f}s", flush=True)
    
    model_payload = {
        'extractor': extractor,
        'classifier': clf,
        'classes': classes,
        'metadata': {
            'num_samples': len(records),
            'num_features': X.shape[1],
            'classes': classes,
            'trained_at': time.strftime("%Y-%m-%d %H:%M:%S")
        }
    }
    
    MODEL_OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    joblib.dump(model_payload, MODEL_OUTPUT_PATH, compress=3)
    print(f"Model saved successfully to {MODEL_OUTPUT_PATH} ({MODEL_OUTPUT_PATH.stat().st_size / (1024*1024):.1f} MB)", flush=True)

if __name__ == '__main__':
    main()
