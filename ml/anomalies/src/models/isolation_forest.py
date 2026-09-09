import logging
import numpy as np
import pandas as pd
from typing import Dict, List, Any, Tuple
from sklearn.ensemble import IsolationForest
from sklearn.preprocessing import RobustScaler

logger = logging.getLogger(__name__)

class IsolationForestAnomalyDetector:
    def __init__(
        self,
        n_estimators: int = 150,
        contamination: float = 0.05,
        random_state: int = 42,
    ):
        self.n_estimators = n_estimators
        self.contamination = contamination
        self.random_state = random_state
        self.scaler = RobustScaler()
        self.model = IsolationForest(
            n_estimators=self.n_estimators,
            contamination=self.contamination,
            random_state=self.random_state,
            n_jobs=-1,
        )
        self.feature_names: List[str] = []
        self.train_medians: np.ndarray = np.array([])
        self.train_iqrs: np.ndarray = np.array([])

    def fit(self, X: pd.DataFrame):
        """
        Fit scaler, Isolation Forest, and baseline distribution statistics.
        """
        self.feature_names = list(X.columns)
        X_vals = X.values.astype(np.float64)

        # Record distribution baselines for feature attribution
        self.train_medians = np.nanmedian(X_vals, axis=0)
        q75 = np.nanpercentile(X_vals, 75, axis=0)
        q25 = np.nanpercentile(X_vals, 25, axis=0)
        self.train_iqrs = np.maximum(1e-6, q75 - q25)

        X_scaled = self.scaler.fit_transform(X_vals)
        self.model.fit(X_scaled)
        logger.info(f"Fitted IsolationForest on {len(X)} samples with {len(self.feature_names)} features.")
        return self

    def score(self, X: pd.DataFrame) -> np.ndarray:
        """
        Return calibrated anomaly score in [0, 1], where 1.0 is highest anomaly.
        Scikit-learn decision_function returns negative values for anomalies, positive for inliers.
        We invert and apply min-max / sigmoid mapping.
        """
        X_scaled = self.scaler.transform(X[self.feature_names].values.astype(np.float64))
        raw_scores = self.model.decision_function(X_scaled)
        # Shift and scale raw scores so lower raw_score maps to higher anomaly score in [0, 1]
        # In scikit-learn, decision_function is around -0.3 to +0.3
        anomaly_scores = 1.0 / (1.0 + np.exp(raw_scores * 8.0))
        return anomaly_scores

    def explain_anomaly(self, sample_row: pd.Series, top_k: int = 4) -> List[Dict[str, Any]]:
        """
        Identify top contributing features for an anomalous sample based on IQR deviation from median.
        """
        vals = sample_row[self.feature_names].values.astype(np.float64)
        deviations = (vals - self.train_medians) / self.train_iqrs
        abs_dev = np.abs(deviations)

        top_indices = np.argsort(abs_dev)[::-1][:top_k]
        explanations = []
        for idx in top_indices:
            feat = self.feature_names[idx]
            explanations.append({
                "feature": feat,
                "current_value": float(vals[idx]),
                "baseline_median": float(self.train_medians[idx]),
                "z_deviation": float(deviations[idx]),
            })
        return explanations
