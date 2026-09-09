"""
PreVerifier: Dual-Head Verification Prioritizer & Expected Utility Queue Ranker.
"""
import random
import math
from typing import Dict, Any, List, Optional
from sklearn.ensemble import HistGradientBoostingClassifier


class PreVerifier:
    def __init__(self, random_state: int = 42):
        self.head_a = HistGradientBoostingClassifier(
            max_iter=100,
            learning_rate=0.08,
            max_depth=5,
            min_samples_leaf=20,
            random_state=random_state,
        )
        self.head_b = HistGradientBoostingClassifier(
            max_iter=100,
            learning_rate=0.08,
            max_depth=5,
            min_samples_leaf=20,
            random_state=random_state + 1,
        )
        self.feature_names = [
            "seconds_since_first_sighting",
            "seconds_since_last_sighting",
            "log_seconds_since_last",
            "announce_to_get_peers_ratio",
            "log_sighting_count",
            "sighting_interarrival_mean",
            "prior_attempt_count",
            "seconds_since_last_attempt",
            "hour_of_day_sin",
            "hour_of_day_cos",
            "day_of_week",
        ]
        self.is_fitted = False

    def fit(self, X: List[List[float]], y_a: List[int], y_b: List[int]):
        self.head_a.fit(X, y_a)
        self.head_b.fit(X, y_b)
        self.is_fitted = True

    def predict_probabilities(self, X: List[List[float]]) -> List[Dict[str, float]]:
        if not self.is_fitted:
            # Fallback heuristic prior before fitting
            return [{"p_verify_15m": 0.25, "p_verify_72h": 0.40} for _ in X]

        probs_a = self.head_a.predict_proba(X)[:, 1]
        probs_b = self.head_b.predict_proba(X)[:, 1]
        results = []
        for pa, pb in zip(probs_a, probs_b):
            results.append({
                "p_verify_15m": float(pa),
                "p_verify_72h": float(pb),
            })
        return results

    @staticmethod
    def compute_expected_utility(
        p_verify_15m: float,
        p_verify_72h: float,
        prior_attempts: int = 0,
        novelty_score: float = 1.0,
        expected_cost_seconds: float = 10.0,
        cost_floor: float = 2.0,
        exploration_budget: float = 0.10,
    ) -> float:
        """
        Expected Utility Queue Scoring Formula:
        Utility = (p_15m * Catalog Value) / max(cost, floor) + explore + long_tail - retry_penalty
        """
        # 10% randomized exploration traffic reservation
        if random.random() < exploration_budget:
            # Exploration slot
            return 100.0 + random.random() * 50.0

        catalog_value = min(2.0, max(0.5, novelty_score))
        effective_cost = max(expected_cost_seconds, cost_floor)
        base_utility = (p_verify_15m * catalog_value * 100.0) / effective_cost

        # Retry penalty (diminishing return on chronic failures)
        retry_penalty = prior_attempts * 2.5

        # Long-tail bonus for viable swarms with high 72h survival
        long_tail_bonus = 5.0 * p_verify_72h

        return max(0.0, base_utility + long_tail_bonus - retry_penalty)
