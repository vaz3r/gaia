import logging
from datetime import datetime, timedelta, timezone
from typing import Optional, List, Dict, Any
from fastapi import FastAPI, Query, HTTPException
from pydantic import BaseModel

from src.data.postgres_extractor import PostgresExtractor
from src.data.feature_pipeline import FeaturePipeline
from src.pipeline.detector import AnomalyDetector

logger = logging.getLogger(__name__)

app = FastAPI(
    title="Gaia Crawler Operations Anomaly Detection API",
    description="Real-time ML anomaly detection for 24/7 DHT crawler operations",
    version="1.0.0",
)

detector = None

@app.on_event("startup")
def load_detector():
    global detector
    detector = AnomalyDetector()

@app.get("/health")
def health():
    return {"status": "ok", "models_loaded": detector.iso_forest is not None and detector.autoencoder is not None}

@app.get("/api/anomalies/recent")
def get_recent_anomalies(
    hours: int = Query(24, ge=1, le=168, description="Number of past hours to evaluate"),
    min_score: float = Query(0.55, ge=0.0, le=1.0, description="Minimum anomaly score threshold"),
):
    """
    Fetch the latest telemetry from PostgreSQL, compute feature deltas, and return anomaly scores.
    """
    if detector is None:
        raise HTTPException(status_code=503, detail="Anomaly detector not initialized")

    end_time = datetime.now(timezone.utc)
    start_time = end_time - timedelta(hours=hours + 2)

    try:
        extractor = PostgresExtractor()
        raw_df = extractor.extract_raw_metrics(start_time=start_time, end_time=end_time)
        if raw_df.empty:
            return {"status": "no_data", "alerts": []}

        pipeline = FeaturePipeline()
        features_df = pipeline.transform_raw_to_features(raw_df).tail(hours)
        reports = detector.detect(features_df)

        flagged = [r for r in reports if r["anomaly_score"] >= min_score]
        return {
            "time_range": {
                "start": start_time.isoformat(),
                "end": end_time.isoformat(),
            },
            "evaluated_hours": len(reports),
            "flagged_count": len(flagged),
            "alerts": flagged,
            "all_reports": reports,
        }
    except Exception as e:
        logger.error(f"Error evaluating recent telemetry: {e}")
        raise HTTPException(status_code=500, detail=str(e))
