import logging
import joblib
import numpy as np
import pandas as pd
from pathlib import Path
from typing import Dict, List, Any, Optional

from config import MODELS_DIR, ALERT_THRESHOLDS
from src.data.incident_labeler import INCIDENT_MAP

logger = logging.getLogger(__name__)

RECOMMENDATIONS = {
    "DB_LATENCY_SPIKE": (
        "High database latency or blocked worker threads detected. Check slow query statements "
        "in Postgres, inspect lock contention on torrents/verification_jobs, and verify autovacuum status."
    ),
    "DHT_DROP_COLLAPSE": (
        "Severe drop in DHT inbound traffic or routing table collapse. Check UDP socket binding, "
        "verify ISP/firewall UDP rate limiting, and confirm seed node connectivity."
    ),
    "TIMEOUT_CASCADE": (
        "Verification or fetch connection timeout surge detected. Inspect outbound connectivity, "
        "check if crawler IP is blocked by major seedbox providers, or investigate negative cache hit rates."
    ),
    "RESTART_EVENT": (
        "Crawler restart or process crash event detected. Verify host memory limits, OOM logs, and systemd journal."
    ),
    "NORMAL": "All operational telemetry within normal statistical and manifold parameters.",
}

class AnomalyDetector:
    def __init__(self, models_dir: Path = MODELS_DIR):
        self.models_dir = models_dir
        self.iso_forest = None
        self.autoencoder = None
        self.supervised = None
        self._load_models()

    def _load_models(self):
        iso_path = self.models_dir / "isolation_forest.joblib"
        ae_path = self.models_dir / "autoencoder.joblib"
        sup_path = self.models_dir / "supervised_classifier.joblib"

        if iso_path.exists():
            self.iso_forest = joblib.load(iso_path)
        if ae_path.exists():
            self.autoencoder = joblib.load(ae_path)
        if sup_path.exists():
            self.supervised = joblib.load(sup_path)

        if not (self.iso_forest and self.autoencoder):
            logger.warning("Models not fully loaded. Train models first using train.py.")

    def reload_models(self):
        """Hot-reload model artifacts from disk without restarting service."""
        logger.info("Hot-reloading anomaly models from storage...")
        self._load_models()
        logger.info("Hot-reload complete.")

    def detect(self, features_df: pd.DataFrame) -> List[Dict[str, Any]]:
        """
        Analyze feature vectors, compute ensemble anomaly scores, determine severity,
        and provide root-cause explanations.
        """
        if self.iso_forest is None or self.autoencoder is None:
            raise RuntimeError("Models are not loaded. Run training first.")

        if_scores = self.iso_forest.score(features_df)
        ae_scores = self.autoencoder.score(features_df)
        ensemble_scores = 0.5 * if_scores + 0.5 * ae_scores

        supervised_preds = []
        if self.supervised is not None:
            supervised_preds = self.supervised.predict_incident(features_df)

        reports = []
        for i in range(len(features_df)):
            ts = features_df.index[i]
            row = features_df.iloc[i]
            score = float(ensemble_scores[i])

            # Severity classification
            if score >= ALERT_THRESHOLDS["critical"]:
                severity = "CRITICAL"
            elif score >= ALERT_THRESHOLDS["warning"]:
                severity = "WARNING"
            elif score >= ALERT_THRESHOLDS["info"]:
                severity = "INFO"
            else:
                severity = "NORMAL"

            # Root-cause feature attribution
            top_features = self.iso_forest.explain_anomaly(row, top_k=4)

            # Incident type prediction
            incident_info = {"incident_type": "NORMAL", "confidence": 1.0}
            if supervised_preds:
                incident_info = supervised_preds[i]

            incident_type = incident_info["incident_type"]
            if severity == "NORMAL" and incident_type != "NORMAL":
                # Only report incident type if severity is at least INFO or score > 0.50
                if score < 0.50:
                    incident_type = "NORMAL"

            reports.append({
                "timestamp": str(ts),
                "anomaly_score": round(score, 4),
                "isolation_forest_score": round(float(if_scores[i]), 4),
                "autoencoder_score": round(float(ae_scores[i]), 4),
                "severity": severity,
                "predicted_incident": incident_type,
                "incident_confidence": round(float(incident_info.get("confidence", 0)), 4),
                "top_contributing_features": top_features,
                "actionable_guidance": RECOMMENDATIONS.get(incident_type, RECOMMENDATIONS["NORMAL"]),
            })

        return reports
