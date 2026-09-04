#!/usr/bin/env python3
"""
Test MLP classifier on N torrents from PostgreSQL.
Exports results and generates accuracy report.
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "src"))

import psycopg2
import psycopg2.extras
from backends.mlp_backend import MLPBackend
from core.text_builder import build_input_text
from core.feature_extractor import extract_numeric_features

DB_CONFIG = {
    "host": "workspace-production",
    "port": 5432,
    "user": "crawler",
    "dbname": "craw",
    "password": "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b",
    "connect_timeout": 10,
}

MODEL_PATH = "data/models/mlp_8k_human/torrent_classifier.joblib"
OUTPUT_DIR = Path("data/test_results")
NUM_TORRENTS = 50000
BATCH_SIZE = 512

# SQL to fetch torrents with full metadata
FETCH_SQL = """
WITH sampled AS (
    SELECT infohash
    FROM torrents
    WHERE files IS NOT NULL
      AND jsonb_array_length(files) > 0
    ORDER BY random()
    LIMIT %s
)
SELECT
    encode(t.infohash, 'hex') AS infohash,
    t.name,
    t.file_count,
    t.total_size,
    t.files AS files_raw
FROM torrents t
JOIN sampled s ON t.infohash = s.infohash
"""


def fetch_torrents(limit: int) -> list[dict]:
    """Fetch random torrents with full file metadata."""
    print(f"Fetching {limit} torrents from PostgreSQL...")
    conn = psycopg2.connect(**DB_CONFIG)
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(FETCH_SQL, (limit,))
            rows = cur.fetchall()
        return [dict(row) for row in rows]
    finally:
        conn.close()


def classify_batch(backend: MLPBackend, torrents: list[dict]) -> list[dict]:
    """Classify a batch of torrents and return results."""
    texts = [build_input_text(t) for t in torrents]
    numeric_features = [extract_numeric_features(t) for t in torrents]
    labels, confidences = backend.predict_labels(texts, numeric_features)
    
    results = []
    for i, torrent in enumerate(torrents):
        results.append({
            "infohash": torrent["infohash"],
            "name": torrent["name"],
            "file_count": torrent["file_count"],
            "total_size": torrent["total_size"],
            "predicted_category": labels[i],
            "confidence": float(confidences[i]),
        })
    return results


def analyze_results(results: list[dict]) -> dict:
    """Analyze classification results."""
    total = len(results)
    
    # Category distribution
    categories = {}
    for r in results:
        cat = r["predicted_category"]
        categories[cat] = categories.get(cat, 0) + 1
    
    # Confidence distribution
    confidences = [r["confidence"] for r in results]
    avg_confidence = sum(confidences) / len(confidences)
    min_confidence = min(confidences)
    max_confidence = max(confidences)
    
    # Confidence buckets
    buckets = {
        "high (>=0.9)": 0,
        "medium (0.7-0.9)": 0,
        "low (0.5-0.7)": 0,
        "very_low (<0.5)": 0,
    }
    for c in confidences:
        if c >= 0.9:
            buckets["high (>=0.9)"] += 1
        elif c >= 0.7:
            buckets["medium (0.7-0.9)"] += 1
        elif c >= 0.5:
            buckets["low (0.5-0.7)"] += 1
        else:
            buckets["very_low (<0.5)"] += 1
    
    # Low confidence examples (most uncertain)
    low_conf = sorted(results, key=lambda x: x["confidence"])[:20]
    
    return {
        "total": total,
        "category_distribution": dict(sorted(categories.items(), key=lambda x: -x[1])),
        "confidence_stats": {
            "average": round(avg_confidence, 4),
            "min": round(min_confidence, 4),
            "max": round(max_confidence, 4),
        },
        "confidence_buckets": buckets,
        "low_confidence_examples": low_conf,
    }


def main():
    # Load model
    print("Loading MLP model...")
    backend = MLPBackend(MODEL_PATH)
    print(f"Loaded model with {len(backend.classes)} classes: {list(backend.classes)}")
    
    # Fetch torrents
    torrents = fetch_torrents(NUM_TORRENTS)
    print(f"Fetched {len(torrents)} torrents")
    
    # Classify in batches
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    all_results = []
    
    start_time = time.time()
    for i in range(0, len(torrents), BATCH_SIZE):
        batch = torrents[i:i + BATCH_SIZE]
        batch_num = i // BATCH_SIZE + 1
        total_batches = (len(torrents) + BATCH_SIZE - 1) // BATCH_SIZE
        
        results = classify_batch(backend, batch)
        all_results.extend(results)
        
        elapsed = time.time() - start_time
        rate = len(all_results) / elapsed if elapsed > 0 else 0
        print(f"Batch {batch_num}/{total_batches}: {len(all_results)}/{len(torrents)} ({rate:.0f} samples/s)")
    
    elapsed = time.time() - start_time
    print(f"\nClassified {len(all_results)} torrents in {elapsed:.1f}s ({len(all_results)/elapsed:.0f} samples/s)")
    
    # Save results
    output_file = OUTPUT_DIR / "test_50k_results.jsonl"
    with open(output_file, "w") as f:
        for r in all_results:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"Results saved to: {output_file}")
    
    # Analyze
    analysis = analyze_results(all_results)
    
    # Save analysis
    analysis_file = OUTPUT_DIR / "test_50k_analysis.json"
    with open(analysis_file, "w") as f:
        json.dump(analysis, f, indent=2, ensure_ascii=False)
    print(f"Analysis saved to: {analysis_file}")
    
    # Print report
    print("\n" + "=" * 60)
    print("CLASSIFICATION REPORT — 50K Torrents")
    print("=" * 60)
    print(f"\nTotal classified: {analysis['total']}")
    print(f"\nCategory distribution:")
    for cat, count in analysis["category_distribution"].items():
        pct = count / analysis["total"] * 100
        print(f"  {cat:<20} {count:>6} ({pct:5.1f}%)")
    print(f"\nConfidence statistics:")
    print(f"  Average: {analysis['confidence_stats']['average']:.3f}")
    print(f"  Min:     {analysis['confidence_stats']['min']:.3f}")
    print(f"  Max:     {analysis['confidence_stats']['max']:.3f}")
    print(f"\nConfidence distribution:")
    for bucket, count in analysis["confidence_buckets"].items():
        pct = count / analysis["total"] * 100
        print(f"  {bucket:<25} {count:>6} ({pct:5.1f}%)")
    print(f"\nLow confidence examples (most uncertain):")
    for ex in analysis["low_confidence_examples"][:10]:
        print(f"  [{ex['confidence']:.3f}] {ex['predicted_category']:<15} {ex['name'][:60]}")
    
    # Export low-confidence items for re-labeling
    low_conf_items = [r for r in all_results if r["confidence"] < 0.7]
    low_conf_file = OUTPUT_DIR / "test_50k_low_conf.jsonl"
    with open(low_conf_file, "w") as f:
        for r in low_conf_items:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"\nLow confidence items (<0.7): {len(low_conf_items)} ({len(low_conf_items)/len(all_results)*100:.1f}%)")
    print(f"Exported to: {low_conf_file}")
    
    # Export infohashes for targeted labeling
    infohashes = [r["infohash"] for r in low_conf_items]
    ih_file = OUTPUT_DIR / "test_50k_low_conf_infohashes.txt"
    with open(ih_file, "w") as f:
        for ih in infohashes:
            f.write(ih + "\n")
    print(f"Infohashes exported to: {ih_file}")
    
    print("\n" + "=" * 60)


if __name__ == "__main__":
    main()
