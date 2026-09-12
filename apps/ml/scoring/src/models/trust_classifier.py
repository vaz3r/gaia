"""
TrustClassifier: Calibrated authenticity and safety classifier.
"""
import joblib
from pathlib import Path
from typing import List, Dict, Any, Optional
import numpy as np
from sklearn.ensemble import HistGradientBoostingClassifier
from src.calibration.calibrator import ProbabilityCalibrator


class TrustClassifier:
    def __init__(self, random_state: int = 42, calibration_method: str = "isotonic"):
        self.model = HistGradientBoostingClassifier(
            max_iter=150,
            learning_rate=0.05,
            max_depth=6,
            min_samples_leaf=30,
            l2_regularization=1.0,
            random_state=random_state,
        )
        self.calibrator = ProbabilityCalibrator(method=calibration_method)
        self.feature_names = [
            "log_total_size",
            "log_file_count",
            "log_num_pieces",
            "piece_length_is_pow2",
            "max_path_depth",
            "mean_path_depth",
            "video_share",
            "audio_share",
            "executable_share",
            "archive_share",
            "doc_share",
            "junk_share",
            "primary_ext_share",
            "extension_entropy",
            "zero_byte_file_ratio",
            "title_length",
            "title_entropy",
            "spam_keyword_count",
            "path_has_non_ascii",
            "deceptive_double_extension",
            "category_is_media",
            "category_is_software",
            "media_executable_mismatch",
            "media_standalone_executable",
            "has_rtlo_spoofing",
            "has_dangerous_script",
            "software_implausible_size",
            "executable_file_count",
        ]
        self.is_trained = False

    def fit(self, X_train: np.ndarray, y_train: np.ndarray, X_cal: np.ndarray, y_cal: np.ndarray):
        """Train classifier and calibrate probabilities on held-out calibration split."""
        self.model.fit(X_train, y_train)
        uncal_cal_probs = self.model.predict_proba(X_cal)[:, 1]
        self.calibrator.fit(uncal_cal_probs, y_cal)
        self.is_trained = True

    def predict_safe_probability(self, X: np.ndarray) -> np.ndarray:
        """Returns calibrated P(safe) in [0.0, 1.0]."""
        if not self.is_trained:
            # High safety prior for unclassified content
            return np.full((len(X),), 0.90)
        raw_probs = self.model.predict_proba(X)[:, 1]
        return self.calibrator.predict(raw_probs)

    def save(self, output_path: Path):
        output_path.parent.mkdir(parents=True, exist_ok=True)
        joblib.dump(self, output_path)

    @classmethod
    def load(cls, model_path: Path) -> "TrustClassifier":
        return joblib.load(model_path)
