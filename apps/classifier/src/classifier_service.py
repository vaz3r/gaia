import sys
import re
from pathlib import Path
from typing import Dict, Any, List, Optional
import joblib
import numpy as np

sys.path.append(str(Path(__file__).parent))
from feature_extractor import (
    TorrentFeatureExtractor,
    explain_features,
    extract_extension,
    EXT_CATEGORIES,
    RE_TV,
    RE_ANIME,
    RE_ADULT,
    RE_AUDIOBOOK,
    RE_BOOK,
    RE_DOCU,
    RE_GAME,
    RE_APP,
    RE_MOVIE,
    RE_MUSIC,
)
from model_manager import get_active_model_path, get_active_model_info

# Mapping predicted categories to their domain regex rule
CATEGORY_REGEX_RULES = {
    "Adult": RE_ADULT,
    "Anime": RE_ANIME,
    "Television": RE_TV,
    "Audiobooks": RE_AUDIOBOOK,
    "Books & Learning": RE_BOOK,
    "Documentaries": RE_DOCU,
    "Games": RE_GAME,
    "Applications": RE_APP,
    "Movies": RE_MOVIE,
    "Music": RE_MUSIC,
}

# JAV (Japanese Adult Video) scene code regex: e.g. SNIS-851, IPX-292, ABP-108, FC2-PPV
RE_JAV = re.compile(r'\b[A-Za-z]{2,6}[-_ ]\d{3,5}\b|\bFC2\b', re.IGNORECASE)

CATEGORY_EXT_MAPPING = {
    "Audiobooks": "audiobook",
    "Books & Learning": "ebook",
    "Games": "game_rom",
    "Applications": "software",
    "Music": "audio",
}


def compute_effective_thresholds(
    item: Dict[str, Any],
    top_cat: str,
    base_conf_threshold: float = 0.65,
    base_margin_threshold: float = 0.35,
) -> tuple[float, float, bool, bool]:
    """
    Computes manifest-aware adaptive thresholds and rule confirmation.
    Returns: (eff_conf_thresh, eff_margin_thresh, has_manifest, rule_matched)
    """
    files = item.get("files")
    has_manifest = False
    if files:
        if isinstance(files, str):
            try:
                import json
                parsed = json.loads(files)
                has_manifest = bool(isinstance(parsed, list) and len(parsed) > 0)
            except Exception:
                has_manifest = False
        elif isinstance(files, list):
            has_manifest = len(files) > 0

    name = str(item.get("name") or "")
    rule = CATEGORY_REGEX_RULES.get(top_cat)
    rule_matched = bool(rule and rule.search(name))

    # Supplemental heuristics for adult and file extensions
    if not rule_matched:
        if top_cat == "Adult" and RE_JAV.search(name):
            rule_matched = True
        else:
            cat_target = CATEGORY_EXT_MAPPING.get(top_cat)
            if cat_target:
                name_ext = extract_extension(name)
                if name_ext and name_ext in EXT_CATEGORIES.get(cat_target, set()):
                    rule_matched = True

    if has_manifest:
        # Rich multi-file manifest: apply standard tight quality gates
        eff_conf = base_conf_threshold
        eff_margin = base_margin_threshold
    else:
        # Single-file / sparse manifest: lower base thresholds
        if rule_matched:
            # Strong domain scene / regex pattern confirmed top category
            eff_conf = min(base_conf_threshold, 0.40)
            eff_margin = min(base_margin_threshold, 0.15)
        else:
            eff_conf = min(base_conf_threshold, 0.50)
            eff_margin = min(base_margin_threshold, 0.20)

    return eff_conf, eff_margin, has_manifest, rule_matched


def extract_gating_vector(p: np.ndarray, item: Dict[str, Any]) -> List[float]:
    sorted_p = np.sort(p)[::-1]
    top1_p = float(sorted_p[0])
    top2_p = float(sorted_p[1]) if len(sorted_p) > 1 else 0.0
    top3_p = float(sorted_p[2]) if len(sorted_p) > 2 else 0.0
    margin1_2 = top1_p - top2_p
    margin2_3 = top2_p - top3_p
    ent = float(-np.sum(p * np.log(p + 1e-12)))

    total_size = float(item.get("total_size") or 0.0)
    file_count = float(item.get("file_count") or 1.0)
    log_size = float(np.log10(max(total_size, 1.0)) / 12.0)
    log_count = float(np.log10(max(file_count, 1.0)) / 5.0)
    is_single = 1.0 if file_count <= 1 else 0.0

    integrity_score = float(item.get("integrity_score") if item.get("integrity_score") is not None else 100.0) / 100.0
    model_safe_prob = float(item.get("model_safe_probability") if item.get("model_safe_probability") is not None else 1.0)
    metadata_quality = float(item.get("metadata_quality_score") if item.get("metadata_quality_score") is not None else 100.0) / 100.0

    return [
        top1_p,
        margin1_2,
        margin2_3,
        ent,
        log_size,
        log_count,
        is_single,
        integrity_score,
        model_safe_prob,
        metadata_quality,
    ]


class TorrentClassifierService:
    _instance: Optional["TorrentClassifierService"] = None

    def __init__(self):
        self.model_path = get_active_model_path()
        self._load_model()

    def _load_model(self):
        payload = joblib.load(self.model_path)
        self.extractor: TorrentFeatureExtractor = payload["extractor"]
        self.classifier = payload["classifier"]
        self.gating_model = payload.get("gating_model")
        self.classes = [str(c) for c in payload["classes"]]
        self.metadata = payload.get("metadata", {})
        self.active_info = get_active_model_info()

    def check_reload(self):
        """Hot-reload if active model path or timestamp changed."""
        latest_path = get_active_model_path()
        if latest_path != self.model_path:
            self.model_path = latest_path
            self._load_model()

    @classmethod
    def get_instance(cls) -> "TorrentClassifierService":
        if cls._instance is None:
            cls._instance = TorrentClassifierService()
        else:
            cls._instance.check_reload()
        return cls._instance

    def classify_single(
        self,
        item: Dict[str, Any],
        confidence_threshold: float = 0.65,
        margin_threshold: float = 0.35,
        adaptive_thresholds: bool = True,
    ) -> Dict[str, Any]:
        """Classify a single torrent dict and provide comprehensive diagnostic breakdown."""
        self.check_reload()

        # 1. Integrity / Scoring ML suppression check
        policy_action = str(item.get("policy_action") or "").upper()
        risk_tier = str(item.get("risk_tier") or "").upper()
        if policy_action == "SUPPRESS" or risk_tier == "BLOCKED":
            return {
                "predicted_category": "Other",
                "confidence": 1.0,
                "margin": 1.0,
                "top2_category": "None",
                "top2_confidence": 0.0,
                "needs_review": False,
                "review_reason": "Suppressed by Integrity ML: Payload marked as malicious/deceptive/blocked.",
                "review_type": "suppressed_by_integrity",
                "probabilities": [{"category": "Other", "probability": 1.0}],
                "features": explain_features(item),
                "model_version": self.active_info.get("version", "v5"),
                "model_classes": self.classes,
            }

        X = self.extractor.transform([item])
        probas = self.classifier.predict_proba(X)[0]

        ranked = [
            {"category": str(cat), "probability": round(float(prob), 4)}
            for cat, prob in sorted(zip(self.classes, probas), key=lambda x: -x[1])
        ]

        top1 = ranked[0]
        top2 = ranked[1] if len(ranked) > 1 else {"category": "None", "probability": 0.0}
        margin = round(top1["probability"] - top2["probability"], 4)
        name = str(item.get("name") or "")

        # 2. Gating ML decision vs legacy threshold rules
        if self.gating_model is not None:
            g_vec = extract_gating_vector(probas, item)
            p_correct = float(self.gating_model.predict_proba([g_vec])[0, 1])

            # Adult Sanity Lock: Prevent clean foreign titles from defaulting to Adult
            if top1["category"] == "Adult" and not (RE_ADULT.search(name) or RE_JAV.search(name)):
                p_correct = min(p_correct, 0.40)

            if p_correct >= 0.95:
                needs_review = False
                review_reason = None
                review_type = "accepted"
            else:
                needs_review = True
                review_reason = f"Gating ML flagged: P(Correct) is {p_correct*100:.1f}% (below 95% certitude threshold)."
                review_type = "low_confidence"
            eff_conf, eff_margin = 0.95, 0.0
            has_manifest, rule_matched = bool(item.get("files")), False
        else:
            p_correct = None
            if adaptive_thresholds:
                eff_conf, eff_margin, has_manifest, rule_matched = compute_effective_thresholds(
                    item, top1["category"], confidence_threshold, margin_threshold
                )
            else:
                eff_conf, eff_margin = confidence_threshold, margin_threshold
                has_manifest, rule_matched = bool(item.get("files")), False

            is_low_conf = top1["probability"] < eff_conf
            is_ambiguous = (not is_low_conf) and (margin < eff_margin)
            needs_review = is_low_conf or is_ambiguous

            if is_low_conf:
                review_reason = f"Low Confidence: Top probability is {top1['probability']*100:.1f}% (below threshold {eff_conf*100:.0f}%)."
            elif is_ambiguous:
                review_reason = f"Ambiguous Boundary: Margin is only {margin*100:.1f}% between '{top1['category']}' and '{top2['category']}'."
            else:
                review_reason = None
            review_type = "low_confidence" if is_low_conf else ("ambiguous" if is_ambiguous else "accepted")

        feature_info = explain_features(item)

        return {
            "predicted_category": top1["category"],
            "confidence": top1["probability"],
            "margin": margin,
            "gating_p_correct": round(p_correct, 4) if p_correct is not None else None,
            "top2_category": top2["category"],
            "top2_confidence": top2["probability"],
            "needs_review": needs_review,
            "review_reason": review_reason,
            "review_type": review_type,
            "probabilities": ranked,
            "features": feature_info,
            "effective_thresholds": {
                "confidence": eff_conf,
                "margin": eff_margin,
                "has_manifest": has_manifest,
                "rule_matched": rule_matched,
            },
            "model_version": self.active_info.get("version", "v6"),
            "model_classes": self.classes,
        }

    def classify_batch(
        self,
        items: List[Dict[str, Any]],
        confidence_threshold: float = 0.65,
        margin_threshold: float = 0.35,
        adaptive_thresholds: bool = True,
    ) -> List[Dict[str, Any]]:
        """Classify a batch of torrent dicts efficiently with Gating ML / Integrity check."""
        if not items:
            return []

        self.check_reload()
        X = self.extractor.transform(items)
        probas = self.classifier.predict_proba(X)

        results = []
        classes_arr = np.array(self.classes)

        # Batch compute gating features if gating_model available
        if self.gating_model is not None:
            g_vecs = [extract_gating_vector(probas[i], items[i]) for i in range(len(items))]
            p_correct_batch = self.gating_model.predict_proba(g_vecs)[:, 1]
        else:
            p_correct_batch = None

        for i, item in enumerate(items):
            policy_action = str(item.get("policy_action") or "").upper()
            risk_tier = str(item.get("risk_tier") or "").upper()

            # Integrity Suppression Check
            if policy_action == "SUPPRESS" or risk_tier == "BLOCKED":
                results.append({
                    "predicted_category": "Other",
                    "confidence": 1.0,
                    "margin": 1.0,
                    "top2_category": "None",
                    "top2_confidence": 0.0,
                    "needs_review": False,
                    "review_type": "suppressed_by_integrity",
                    "meta": {
                        "margin": 1.0,
                        "review_type": "suppressed_by_integrity",
                        "top2": {"category": "None", "confidence": 0.0},
                        "model_version": self.active_info.get("version", "v5"),
                    },
                })
                continue

            p = probas[i]
            sorted_indices = np.argsort(p)[::-1]
            top1_idx = sorted_indices[0]
            top2_idx = sorted_indices[1] if len(sorted_indices) > 1 else top1_idx

            top1_cat = str(classes_arr[top1_idx])
            top1_p = float(p[top1_idx])
            top2_cat = str(classes_arr[top2_idx])
            top2_p = float(p[top2_idx])
            margin = top1_p - top2_p
            name = str(item.get("name") or "")

            if p_correct_batch is not None:
                p_corr = float(p_correct_batch[i])
                # Adult Sanity Lock
                if top1_cat == "Adult" and not (RE_ADULT.search(name) or RE_JAV.search(name)):
                    p_corr = min(p_corr, 0.40)

                needs_review = p_corr < 0.95
                review_type = "accepted" if not needs_review else "low_confidence"
                eff_conf = 0.95
                eff_margin = 0.0
                has_manifest = bool(item.get("files"))
                rule_matched = False
            else:
                if adaptive_thresholds:
                    eff_conf, eff_margin, has_manifest, rule_matched = compute_effective_thresholds(
                        item, top1_cat, confidence_threshold, margin_threshold
                    )
                else:
                    eff_conf, eff_margin = confidence_threshold, margin_threshold
                    has_manifest, rule_matched = bool(item.get("files")), False

                is_low_conf = top1_p < eff_conf
                is_ambiguous = (not is_low_conf) and (margin < eff_margin)
                needs_review = is_low_conf or is_ambiguous
                review_type = "low_confidence" if is_low_conf else ("ambiguous" if is_ambiguous else "accepted")

            results.append({
                "predicted_category": top1_cat,
                "confidence": round(top1_p, 4),
                "margin": round(margin, 4),
                "gating_p_correct": round(p_corr, 4) if p_correct_batch is not None else None,
                "top2_category": top2_cat,
                "top2_confidence": round(top2_p, 4),
                "needs_review": bool(needs_review),
                "review_type": review_type,
                "meta": {
                    "margin": round(margin, 4),
                    "review_type": review_type,
                    "top2": {"category": top2_cat, "confidence": round(top2_p, 4)},
                    "effective_thresholds": {
                        "conf": eff_conf,
                        "margin": eff_margin,
                        "has_manifest": has_manifest,
                        "rule_matched": rule_matched,
                    },
                    "model_version": self.active_info.get("version", "v6"),
                },
            })

        return results
