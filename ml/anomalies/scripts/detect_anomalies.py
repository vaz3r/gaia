#!/usr/bin/env python3
import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import argparse
import pandas as pd
from datetime import datetime, timedelta, timezone
from config import DATA_DIR
from src.data.postgres_extractor import PostgresExtractor
from src.data.feature_pipeline import FeaturePipeline
from src.pipeline.detector import AnomalyDetector

def main():
    parser = argparse.ArgumentParser(description="Run crawler anomaly detection")
    parser.add_argument("--recent-hours", type=int, default=24, help="Analyze last N hours from Postgres")
    parser.add_argument("--from-cache", action="store_true", help="Analyze all historical hours from cached parquet")
    parser.add_argument("--min-score", type=float, default=0.55, help="Filter results with anomaly score >= min-score")
    args = parser.parse_args()

    detector = AnomalyDetector()

    if args.from_cache:
        cache_file = DATA_DIR / "hourly_features.parquet"
        if not cache_file.exists():
            print(f"Cache file {cache_file} not found. Run extract_features.py first.")
            sys.exit(1)
        features_df = pd.read_parquet(cache_file)
        print(f"Loaded {len(features_df)} historical hours from cache.")
    else:
        end_time = datetime.now(timezone.utc)
        start_time = end_time - timedelta(hours=args.recent_hours + 2)  # Extra padding for deltas
        print(f"Querying Postgres for telemetry between {start_time.isoformat()} and {end_time.isoformat()}...")
        
        extractor = PostgresExtractor()
        raw_df = extractor.extract_raw_metrics(start_time=start_time, end_time=end_time)
        if raw_df.empty:
            print("No raw metrics returned for time window.")
            sys.exit(0)

        pipeline = FeaturePipeline()
        features_df = pipeline.transform_raw_to_features(raw_df)
        # Take requested window
        features_df = features_df.tail(args.recent_hours)

    print(f"Running detection on {len(features_df)} time intervals...\n")
    reports = detector.detect(features_df)

    flagged = [r for r in reports if r["anomaly_score"] >= args.min_score]

    print("=" * 90)
    print(f"{'TIMESTAMP':<26} {'SCORE':<7} {'SEVERITY':<10} {'INCIDENT':<20} {'CONF':<6}")
    print("=" * 90)

    for r in reports:
        if r["anomaly_score"] < args.min_score:
            continue
        ts_str = r["timestamp"][:19]
        score_str = f"{r['anomaly_score']:.3f}"
        sev = r["severity"]
        inc = r["predicted_incident"]
        conf = f"{r['incident_confidence']:.2f}"
        print(f"{ts_str:<26} {score_str:<7} {sev:<10} {inc:<20} {conf:<6}")

    print("=" * 90)
    print(f"Total time intervals analyzed: {len(reports)}")
    print(f"Total flagged alerts (score >= {args.min_score}): {len(flagged)}")

    # Print detailed root causes for highest anomalies
    if flagged:
        print("\n--- Detailed Root Cause Explanations for Top Alerts ---")
        top_alerts = sorted(flagged, key=lambda x: x["anomaly_score"], reverse=True)[:5]
        for alert in top_alerts:
            print(f"\n[ALERT] {alert['timestamp']} | Score: {alert['anomaly_score']} | Severity: {alert['severity']}")
            print(f"  Incident Category: {alert['predicted_incident']} (Confidence: {alert['incident_confidence']})")
            print(f"  Guidance: {alert['actionable_guidance']}")
            print("  Top Contributing Feature Deviations:")
            for feat in alert["top_contributing_features"]:
                print(f"    * {feat['feature']}: val={feat['current_value']:.2f} (median={feat['baseline_median']:.2f}, z={feat['z_deviation']:+.2f})")

if __name__ == "__main__":
    main()
