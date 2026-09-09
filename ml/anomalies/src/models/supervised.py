import logging
import numpy as np
import pandas as pd
from typing import Dict, List, Any, Tuple
from sklearn.ensemble import RandomForestClassifier
from src.data.incident_labeler import INCIDENT_MAP

logger = logging.getLogger(__name__)

class SupervisedIncidentClassifier:
    """
    Supervised model trained on labeled operational incidents to categorize root causes
    (DB latency spikes, DHT collapse, timeout cascades, restarts).
    """
    def __init__(self, n_estimators: int = 100, random_state: int = 42):
        self.n_estimators = n_estimators
        self.random_state = random_state
        self.model = RandomForestClassifier(
            n_estimators=self.n_estimators,
            class_weight="balanced",
            random_state=self.random_state,
            n_jobs=-1,
        )
        self.feature_names: List[str] = []
        self.classes_: List[int] = []

    def fit(self, X: pd.DataFrame, y: pd.Series):
        self.feature_names = list(X.columns)
        self.model.fit(X[self.feature_names], y)
        self.classes_ = list(self.model.classes_)
        logger.info(f"Fitted SupervisedIncidentClassifier on {len(X)} samples with classes {self.classes_}.")
        return self

    def predict_incident(self, X: pd.DataFrame) -> List[Dict[str, Any]]:
        """
        Return predicted incident label, human-readable name, and probability distribution.
        """
        preds = self.model.predict(X[self.feature_names])
        probs = self.model.predict_proba(X[self.feature_names])

        results = []
        for i in range(len(X)):
            pred_class = int(preds[i])
            prob_dict = {
                INCIDENT_MAP.get(c, str(c)): float(probs[i][idx])
                for idx, c in enumerate(self.classes_)
            }
            results.append({
                "predicted_code": pred_class,
                "incident_type": INCIDENT_MAP.get(pred_class, "UNKNOWN"),
                "confidence": float(np.max(probs[i])),
                "probabilities": prob_dict,
            })
        return results

    def get_feature_importances(self) -> List[Tuple[str, float]]:
        importances = self.model.feature_importances_
        sorted_indices = np.argsort(importances)[::-1]
        return [(self.feature_names[i], float(importances[i])) for i in sorted_indices]
