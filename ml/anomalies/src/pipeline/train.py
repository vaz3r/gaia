import logging
import joblib
import pandas as pd
from pathlib import Path
from typing import Dict, Any

from config import DATA_DIR, MODELS_DIR
from src.data.postgres_extractor import PostgresExtractor
from src.data.log_extractor import LogExtractor
from src.data.feature_pipeline import FeaturePipeline
from src.data.incident_labeler import IncidentLabeler
from src.models.isolation_forest import IsolationForestAnomalyDetector
from src.models.autoencoder import AutoencoderAnomalyDetector
from src.models.supervised import SupervisedIncidentClassifier

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger(__name__)

def train_all_models(save_dir: Path = MODELS_DIR) -> Dict[str, Any]:
    features_path = DATA_DIR / "hourly_features.parquet"
    labels_path = DATA_DIR / "incident_labels.parquet"

    # Step 1: Ensure dataset is available
    if not features_path.exists():
        logger.info("Features cache not found. Extracting from Postgres...")
        extractor = PostgresExtractor()
        raw_df = extractor.extract_raw_metrics()
        pipeline = FeaturePipeline()
        features_df = pipeline.transform_raw_to_features(raw_df)
        features_df.to_parquet(features_path)
        logger.info(f"Saved features to {features_path}")
    else:
        logger.info(f"Loading cached features from {features_path}")
        features_df = pd.read_parquet(features_path)

    # Step 2: Ensure labels are available
    if not labels_path.exists():
        logger.info("Generating incident ground truth labels...")
        extractor = PostgresExtractor()
        restarts = extractor.extract_restart_events()
        
        log_client = LogExtractor()
        slow_queries = log_client.extract_db_latency_incidents()
        
        labeler = IncidentLabeler()
        labels = labeler.label_dataset(features_df, restarts, slow_queries)
        pd.DataFrame({"label": labels}).to_parquet(labels_path)
    else:
        logger.info(f"Loading cached labels from {labels_path}")
        labels_df = pd.read_parquet(labels_path)
        labels = labels_df["incident_label"] if "incident_label" in labels_df.columns else labels_df.iloc[:, 0]

    logger.info(f"Training dataset size: {len(features_df)} hourly vectors across {features_df.shape[1]} features.")

    # Step 3: Train Unsupervised Isolation Forest
    logger.info("Training Isolation Forest...")
    iso_forest = IsolationForestAnomalyDetector()
    iso_forest.fit(features_df)
    iso_path = save_dir / "isolation_forest.joblib"
    joblib.dump(iso_forest, iso_path)
    logger.info(f"Saved Isolation Forest model to {iso_path}")

    # Step 4: Train Unsupervised Autoencoder
    logger.info("Training Autoencoder...")
    autoencoder = AutoencoderAnomalyDetector()
    autoencoder.fit(features_df)
    ae_path = save_dir / "autoencoder.joblib"
    joblib.dump(autoencoder, ae_path)
    logger.info(f"Saved Autoencoder model to {ae_path}")

    # Step 5: Train Supervised Classifier
    logger.info("Training Supervised Incident Classifier...")
    supervised = SupervisedIncidentClassifier()
    supervised.fit(features_df, labels)
    sup_path = save_dir / "supervised_classifier.joblib"
    joblib.dump(supervised, sup_path)
    logger.info(f"Saved Supervised Classifier model to {sup_path}")

    # Step 6: Summary evaluation
    if_scores = iso_forest.score(features_df)
    ae_scores = autoencoder.score(features_df)
    ensemble_scores = 0.5 * if_scores + 0.5 * ae_scores

    anomalies_found = (ensemble_scores >= 0.70).sum()
    logger.info(f"Training Complete! Found {anomalies_found}/{len(features_df)} anomalous hours (score >= 0.70).")

    return {
        "dataset_size": len(features_df),
        "anomalies_detected": int(anomalies_found),
        "models": {
            "isolation_forest": str(iso_path),
            "autoencoder": str(ae_path),
            "supervised_classifier": str(sup_path),
        },
    }

if __name__ == "__main__":
    train_all_models()
