#!/usr/bin/env python3
import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import logging
import pandas as pd
from config import DATA_DIR
from src.data.postgres_extractor import PostgresExtractor
from src.data.log_extractor import LogExtractor
from src.data.feature_pipeline import FeaturePipeline
from src.data.incident_labeler import IncidentLabeler

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger(__name__)

def main():
    logger.info("--- Step 1: Extracting Raw Metrics from PostgreSQL ---")
    pg_extractor = PostgresExtractor()
    raw_df = pg_extractor.extract_raw_metrics()
    
    logger.info("--- Step 2: Feature Engineering (Resets, Aggregation, Ratios) ---")
    pipeline = FeaturePipeline()
    features_df = pipeline.transform_raw_to_features(raw_df)

    parquet_out = DATA_DIR / "hourly_features.parquet"
    csv_out = DATA_DIR / "hourly_features.csv"
    features_df.to_parquet(parquet_out)
    features_df.to_csv(csv_out)
    logger.info(f"Saved {len(features_df)} feature vectors to {parquet_out} and {csv_out}")

    logger.info("--- Step 3: Ground Truth Incident Labeling ---")
    restarts = pg_extractor.extract_restart_events()
    log_extractor = LogExtractor()
    slow_queries = log_extractor.extract_db_latency_incidents()

    labeler = IncidentLabeler()
    labels = labeler.label_dataset(features_df, restarts, slow_queries)
    labels_df = pd.DataFrame({"incident_label": labels})
    labels_df.to_parquet(DATA_DIR / "incident_labels.parquet")
    labels_df.to_csv(DATA_DIR / "incident_labels.csv")
    logger.info(f"Saved incident labels to {DATA_DIR / 'incident_labels.parquet'}")
    print("\n[SUCCESS] Data extraction and feature pipeline completed successfully.")

if __name__ == "__main__":
    main()
