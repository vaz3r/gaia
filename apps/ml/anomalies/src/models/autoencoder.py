import logging
import numpy as np
import pandas as pd
from typing import Dict, List, Any, Tuple
from sklearn.neural_network import MLPRegressor
from sklearn.preprocessing import StandardScaler

logger = logging.getLogger(__name__)

class AutoencoderAnomalyDetector:
    """
    Reconstruction-based anomaly detector using an autoencoder neural network.
    Learns low-dimensional manifold of normal crawler operations (e.g. diurnal cycles).
    Anomalies manifest as high reconstruction error.
    """
    def __init__(
        self,
        hidden_layer_sizes: Tuple[int, ...] = (16, 8, 16),
        max_iter: int = 300,
        random_state: int = 42,
    ):
        self.hidden_layer_sizes = hidden_layer_sizes
        self.max_iter = max_iter
        self.random_state = random_state
        self.scaler = StandardScaler()
        self.model = MLPRegressor(
            hidden_layer_sizes=self.hidden_layer_sizes,
            activation="relu",
            solver="adam",
            max_iter=self.max_iter,
            random_state=self.random_state,
            early_stopping=True,
            validation_fraction=0.15,
        )
        self.feature_names: List[str] = []
        self.error_threshold_95: float = 1.0

    def fit(self, X: pd.DataFrame):
        self.feature_names = list(X.columns)
        X_vals = X[self.feature_names].values.astype(np.float64)
        X_scaled = self.scaler.fit_transform(X_vals)

        # Autoencoder trains to reconstruct its own input: Y = X
        self.model.fit(X_scaled, X_scaled)

        # Determine threshold on training set
        reconstructed = self.model.predict(X_scaled)
        train_mse = np.mean((X_scaled - reconstructed) ** 2, axis=1)
        self.error_threshold_95 = float(np.percentile(train_mse, 95))
        logger.info(f"Fitted Autoencoder on {len(X)} samples. 95th percentile MSE: {self.error_threshold_95:.4f}")
        return self

    def score(self, X: pd.DataFrame) -> np.ndarray:
        """
        Compute reconstruction error and map to [0, 1] anomaly score.
        """
        X_vals = X[self.feature_names].values.astype(np.float64)
        X_scaled = self.scaler.transform(X_vals)
        reconstructed = self.model.predict(X_scaled)
        mse = np.mean((X_scaled - reconstructed) ** 2, axis=1)

        # Normalize score relative to 95th percentile threshold
        # score = 1 - exp(-0.7 * (mse / threshold))
        normalized = 1.0 - np.exp(-0.7 * (mse / max(1e-5, self.error_threshold_95)))
        return np.clip(normalized, 0.0, 1.0)

    def explain_reconstruction_error(self, sample_row: pd.Series, top_k: int = 4) -> List[Dict[str, Any]]:
        """
        Find features with highest individual reconstruction error.
        """
        vals = sample_row[self.feature_names].values.astype(np.float64).reshape(1, -1)
        scaled = self.scaler.transform(vals)
        recon = self.model.predict(scaled)
        feature_errors = (scaled[0] - recon[0]) ** 2

        top_indices = np.argsort(feature_errors)[::-1][:top_k]
        explanations = []
        for idx in top_indices:
            explanations.append({
                "feature": self.feature_names[idx],
                "feature_reconstruction_error": float(feature_errors[idx]),
                "original_value": float(vals[0, idx]),
            })
        return explanations
