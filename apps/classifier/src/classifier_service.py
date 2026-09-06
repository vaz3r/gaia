import sys
from pathlib import Path
from typing import Dict, Any, List, Optional
import joblib
import numpy as np

sys.path.append(str(Path(__file__).parent))
from feature_extractor import TorrentFeatureExtractor, explain_features

DEFAULT_MODEL_PATH = Path(__file__).parent.parent / "models" / "torrent_classifier_v2.joblib"

class TorrentClassifierService:
    _instance: Optional["TorrentClassifierService"] = None

    def __init__(self, model_path: Path = DEFAULT_MODEL_PATH):
        if not model_path.exists():
            raise FileNotFoundError(f"Model file not found at {model_path}")
        
        self.model_path = model_path
        payload = joblib.load(model_path)
        self.extractor: TorrentFeatureExtractor = payload["extractor"]
        self.classifier = payload["classifier"]
        self.classes = [str(c) for c in payload["classes"]]
        self.metadata = payload.get("metadata", {})

    @classmethod
    def get_instance(cls) -> "TorrentClassifierService":
        if cls._instance is None:
            cls._instance = TorrentClassifierService()
        return cls._instance

    def classify_single(
        self,
        item: Dict[str, Any],
        confidence_threshold: float = 0.65,
        margin_threshold: float = 0.35
    ) -> Dict[str, Any]:
        """Classify a single torrent dict and provide comprehensive diagnostic breakdown."""
        X = self.extractor.transform([item])
        probas = self.classifier.predict_proba(X)[0]

        ranked = [
            {"category": str(cat), "probability": round(float(prob), 4)}
            for cat, prob in sorted(zip(self.classes, probas), key=lambda x: -x[1])
        ]

        top1 = ranked[0]
        top2 = ranked[1] if len(ranked) > 1 else {"category": "None", "probability": 0.0}
        margin = round(top1["probability"] - top2["probability"], 4)

        is_low_conf = top1["probability"] < confidence_threshold
        is_ambiguous = (not is_low_conf) and (margin < margin_threshold)
        needs_review = is_low_conf or is_ambiguous

        if is_low_conf:
            review_reason = (
                f"Low Confidence: Top probability is {top1['probability']*100:.1f}% "
                f"(below safety threshold {confidence_threshold*100:.0f}%)."
            )
        elif is_ambiguous:
            review_reason = (
                f"Ambiguous Boundary: Margin is only {margin*100:.1f}% "
                f"(below threshold {margin_threshold*100:.0f}%) between "
                f"'{top1['category']}' ({top1['probability']*100:.1f}%) and "
                f"'{top2['category']}' ({top2['probability']*100:.1f}%)."
            )
        else:
            review_reason = None

        feature_info = explain_features(item)

        return {
            "predicted_category": top1["category"],
            "confidence": top1["probability"],
            "margin": margin,
            "top2_category": top2["category"],
            "top2_confidence": top2["probability"],
            "needs_review": needs_review,
            "review_reason": review_reason,
            "review_type": "low_confidence" if is_low_conf else ("ambiguous" if is_ambiguous else "accepted"),
            "probabilities": ranked,
            "features": feature_info,
            "model_classes": self.classes
        }

    def classify_batch(
        self,
        items: List[Dict[str, Any]],
        confidence_threshold: float = 0.65,
        margin_threshold: float = 0.35
    ) -> List[Dict[str, Any]]:
        """Classify a batch of torrent dicts efficiently."""
        if not items:
            return []

        X = self.extractor.transform(items)
        probas = self.classifier.predict_proba(X)

        results = []
        classes_arr = np.array(self.classes)
        for i, item in enumerate(items):
            p = probas[i]
            sorted_indices = np.argsort(p)[::-1]
            top1_idx = sorted_indices[0]
            top2_idx = sorted_indices[1] if len(sorted_indices) > 1 else top1_idx

            top1_cat = str(classes_arr[top1_idx])
            top1_p = float(p[top1_idx])
            top2_cat = str(classes_arr[top2_idx])
            top2_p = float(p[top2_idx])
            margin = top1_p - top2_p

            is_low_conf = top1_p < confidence_threshold
            is_ambiguous = (not is_low_conf) and (margin < margin_threshold)
            needs_review = is_low_conf or is_ambiguous

            results.append({
                "predicted_category": top1_cat,
                "confidence": round(top1_p, 4),
                "margin": round(margin, 4),
                "needs_review": bool(needs_review),
                "review_type": "low_confidence" if is_low_conf else ("ambiguous" if is_ambiguous else "accepted")
            })

        return results
