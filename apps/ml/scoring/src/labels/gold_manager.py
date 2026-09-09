"""
Gold Benchmark Dataset Manager & Adjudicator.
Manages 3,000 stratified benchmark samples (Set A: 1,800, Set B: 900, Set C: 300).
"""
import os
import json
import logging
from pathlib import Path
from typing import List, Dict, Any, Tuple, Optional
from datetime import datetime

logger = logging.getLogger(__name__)

GOLD_DATA_PATH = Path(__file__).resolve().parent.parent.parent / "data" / "gold_benchmark.json"
GOLD_DATA_PATH.parent.mkdir(parents=True, exist_ok=True)


class GoldBenchmarkManager:
    def __init__(self, data_path: Path = GOLD_DATA_PATH):
        self.data_path = data_path

    def load_benchmark(self) -> List[Dict[str, Any]]:
        """Load benchmark dataset if exists."""
        if not self.data_path.exists():
            return []
        with open(self.data_path, "r", encoding="utf-8") as f:
            return json.load(f)

    def save_benchmark(self, records: List[Dict[str, Any]]) -> None:
        """Persist benchmark records to disk."""
        with open(self.data_path, "w", encoding="utf-8") as f:
            json.dump(records, f, indent=2, default=str)
        logger.info(f"Saved {len(records)} gold benchmark records to {self.data_path}")

    @staticmethod
    def calculate_agreement(
        annotations_a: List[str],
        annotations_b: List[str]
    ) -> Dict[str, Any]:
        """
        Calculates observed raw agreement and per-class agreement.
        Ensures observed agreement >= 90% and highlights any SUPPRESS conflicts.
        """
        assert len(annotations_a) == len(annotations_b), "Annotation lists must have equal length"
        n = len(annotations_a)
        if n == 0:
            return {"error": "Empty dataset"}

        matches = sum(1 for a, b in zip(annotations_a, annotations_b) if a == b)
        observed_agreement = matches / n

        # Class counts
        classes = sorted(list(set(annotations_a) | set(annotations_b)))
        per_class = {}
        suppress_conflicts = []

        for c in classes:
            c_matches = sum(1 for a, b in zip(annotations_a, annotations_b) if a == c and b == c)
            c_total = sum(1 for a, b in zip(annotations_a, annotations_b) if a == c or b == c)
            per_class[c] = round(c_matches / c_total, 4) if c_total > 0 else 1.0

        for idx, (a, b) in enumerate(zip(annotations_a, annotations_b)):
            if (a == "SUPPRESS" or b == "SUPPRESS") and a != b:
                suppress_conflicts.append({
                    "sample_idx": idx,
                    "annotator_a": a,
                    "annotator_b": b
                })

        return {
            "total_samples": n,
            "raw_agreement_rate": round(observed_agreement, 4),
            "meets_90pct_gate": observed_agreement >= 0.90,
            "per_class_agreement": per_class,
            "unresolved_suppress_conflicts": len(suppress_conflicts),
            "suppress_conflicts": suppress_conflicts
        }
