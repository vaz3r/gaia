"""
Probability Calibration and ECE evaluation for Trust Classifier.
Implements Isotonic regression, Platt scaling, and Expected Calibration Error metrics.
"""
import numpy as np
from typing import Dict, Any, Tuple
from sklearn.isotonic import IsotonicRegression
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import brier_score_loss


class ProbabilityCalibrator:
    def __init__(self, method: str = "isotonic"):
        self.method = method
        if method == "isotonic":
            self.calibrator = IsotonicRegression(out_of_bounds="clip")
        else:
            self.calibrator = LogisticRegression()
        self.is_fitted = False

    def fit(self, uncalibrated_probs: np.ndarray, y_true: np.ndarray):
        """Fit calibration mapping on chronologically held-out calibration split."""
        if self.method == "isotonic":
            self.calibrator.fit(uncalibrated_probs, y_true)
        else:
            self.calibrator.fit(uncalibrated_probs.reshape(-1, 1), y_true)
        self.is_fitted = True

    def predict(self, uncalibrated_probs: np.ndarray) -> np.ndarray:
        """Apply calibration mapping."""
        if not self.is_fitted:
            return np.clip(uncalibrated_probs, 0.0, 1.0)
        if self.method == "isotonic":
            return np.clip(self.calibrator.predict(uncalibrated_probs), 0.0, 1.0)
        else:
            return np.clip(self.calibrator.predict_proba(uncalibrated_probs.reshape(-1, 1))[:, 1], 0.0, 1.0)

    @staticmethod
    def compute_calibration_metrics(y_true: np.ndarray, y_prob: np.ndarray, n_bins: int = 10) -> Dict[str, Any]:
        """
        Computes Brier score and Expected Calibration Error (ECE).
        """
        brier = float(brier_score_loss(y_true, y_prob))

        bins = np.linspace(0.0, 1.0, n_bins + 1)
        bin_lowers = bins[:-1]
        bin_uppers = bins[1:]

        ece = 0.0
        bin_details = []

        for lower, upper in zip(bin_lowers, bin_uppers):
            in_bin = (y_prob >= lower) & (y_prob < upper)
            bin_size = np.sum(in_bin)
            if bin_size > 0:
                acc = np.mean(y_true[in_bin])
                conf = np.mean(y_prob[in_bin])
                diff = np.abs(acc - conf)
                ece += (bin_size / len(y_true)) * diff
                bin_details.append({
                    "range": f"[{lower:.1f}, {upper:.1f})",
                    "count": int(bin_size),
                    "mean_predicted_prob": round(float(conf), 4),
                    "empirical_accuracy": round(float(acc), 4),
                    "diff": round(float(diff), 4),
                })

        return {
            "brier_score": round(brier, 4),
            "expected_calibration_error": round(float(ece), 4),
            "meets_ece_gate": ece < 0.05,
            "bins": bin_details,
        }
