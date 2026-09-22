"""
Canonical health scoring formula.

Pure, deterministic, side-effect-free functions for computing health_score,
health_confidence, and health_state from bounded evidence families.

Algorithm version: 2.0.0
"""

from __future__ import annotations

import logging
import math
from dataclasses import dataclass
from enum import Enum
from typing import Optional

logger = logging.getLogger(__name__)

try:
    from .config import ALGORITHM_VERSION
except ImportError:
    from config import ALGORITHM_VERSION


class HealthState(str, Enum):
    UNKNOWN = "UNKNOWN"
    UNVERIFIED = "UNVERIFIED"
    VERIFIED = "VERIFIED"
    STALE = "STALE"


@dataclass(frozen=True)
class EvidenceFamilies:
    """Bounded evidence inputs for the health scoring formula.

    Each field represents the *aggregated* value for its evidence family,
    not raw observation counts. The aggregator is responsible for applying
    saturating functions and selecting the latest relevant event.
    """

    # Family 1: Direct probe success (latest only, highest trust)
    # Age in hours since the most recent successful metadata/probe fetch.
    # None = no successful probe in history.
    direct_success_age_hours: Optional[float] = None

    # Family 2: Confirmed seed (latest only)
    # Age in hours since the most recent seed confirmation.
    # None = no seed confirmation in history.
    seed_age_hours: Optional[float] = None

    # Family 3: Peer count (saturating log function, single latest observation)
    # Max peer count from the most recent peer observation.
    # peer_evidence_age_hours = age of that observation.
    max_recent_peer_count: int = 0
    peer_evidence_age_hours: Optional[float] = None

    # Family 4: DHT sightings (capped recurrence in 12h window)
    # dht_sighting_count_12h = number of distinct DHT sightings in last 12 hours.
    # latest_dht_sighting_age_hours = age of the most recent DHT sighting.
    dht_sighting_count_12h: int = 0
    latest_dht_sighting_age_hours: Optional[float] = None

    # Failure penalty (capped, based on failures after latest success)
    # recent_failures_after_latest_success = count of normalized direct-failure
    #   observations that occurred AFTER the latest direct success, within the
    #   failure lookback window, capped at 3 before decay is applied.
    # latest_failure_age_hours = age of the most recent failure observation.
    recent_failures_after_latest_success: int = 0
    latest_failure_age_hours: Optional[float] = None


@dataclass(frozen=True)
class HealthScoreResult:
    """Output of the health scoring formula."""

    health_score: Optional[int]
    health_confidence: float
    health_state: HealthState
    algorithm_version: str

    # Component breakdown for debugging/calibration
    direct_component: float = 0.0
    seed_component: float = 0.0
    peer_component: float = 0.0
    dht_component: float = 0.0
    failure_penalty: float = 0.0


def decay(age_hours: float, half_life_hours: float) -> float:
    """True half-life exponential decay.

    Returns the fraction of evidence remaining after age_hours, given a
    half_life_hours. At age_hours == half_life_hours, returns exactly 0.5.

    Args:
        age_hours: Elapsed time in hours since the evidence was observed.
                   Negative values (clock skew / future timestamps) are
                   clamped to 0 and a warning is emitted.
        half_life_hours: Time in hours for evidence to decay to 50%.
                        Must be > 0.

    Returns:
        Fraction remaining in (0.0, 1.0].

    Raises:
        ValueError: If half_life_hours <= 0.
    """
    if half_life_hours <= 0:
        raise ValueError(f"half_life_hours must be greater than zero, got {half_life_hours}")
    if age_hours < 0:
        logger.warning(
            "Negative age_hours=%.2f encountered (possible clock skew). Clamping to 0.",
            age_hours,
        )
        age_hours = 0.0
    return math.exp(-math.log(2) * age_hours / half_life_hours)


def _round_half_up(value: float) -> int:
    """Round non-negative float to int using round-half-up (not banker's rounding).

    Python's built-in round() uses banker's rounding (round half to even).
    For health scores, we want conventional round-half-up:
      70.5 -> 71,  71.5 -> 72,  72.5 -> 73
    """
    if value < 0:
        return int(value)  # negative values clamp to 0 at the caller
    return int(value + 0.5)


def compute_health_score(
    evidence: EvidenceFamilies,
    algorithm_version: str = ALGORITHM_VERSION,
) -> HealthScoreResult:
    """Compute health score, confidence, and state from bounded evidence families.

    This is a pure function with no side effects except logging for telemetry
    on anomalous inputs (negative ages, etc.).

    The score is computed as:
        health_score = clamp(0, 100, direct + seed + peer + dht - failure_penalty)

    Each evidence family contributes a bounded, decayed value. No family can
    exceed its weight, and historical observations cannot accumulate unboundedly.

    Args:
        evidence: Bounded evidence inputs (one value per family, not raw rows).
        algorithm_version: Version string for calibration tracking.

    Returns:
        HealthScoreResult with score, confidence, state, and component breakdown.
    """

    # ---- Family 1: Direct probe success (latest only) ----
    direct = 0.0
    if evidence.direct_success_age_hours is not None:
        direct = 40.0 * decay(evidence.direct_success_age_hours, 24)

    # ---- Family 2: Confirmed seed (latest only) ----
    seed = 0.0
    if evidence.seed_age_hours is not None:
        seed = 30.0 * decay(evidence.seed_age_hours, 36)

    # ---- Family 3: Peer count (saturating log, single observation) ----
    peer = 0.0
    if evidence.max_recent_peer_count > 0 and evidence.peer_evidence_age_hours is not None:
        peer_saturated = min(20.0, 7.0 * math.log2(1 + evidence.max_recent_peer_count))
        peer = peer_saturated * decay(evidence.peer_evidence_age_hours, 18)

    # ---- Family 4: DHT sightings (capped recurrence in 12h window) ----
    dht = 0.0
    if evidence.dht_sighting_count_12h > 0 and evidence.latest_dht_sighting_age_hours is not None:
        dht_capped = min(10.0, 2.0 * math.log2(1 + evidence.dht_sighting_count_12h))
        dht = dht_capped * decay(evidence.latest_dht_sighting_age_hours, 12)

    # ---- Failure penalty (capped, decayed) ----
    failure_penalty = 0.0
    if (
        evidence.recent_failures_after_latest_success > 0
        and evidence.latest_failure_age_hours is not None
    ):
        failure_penalty = min(
            30.0, 10.0 * evidence.recent_failures_after_latest_success
        ) * decay(evidence.latest_failure_age_hours, 6)

    # ---- Raw score and clamping ----
    raw_score = direct + seed + peer + dht - failure_penalty
    health_score = max(0, min(100, _round_half_up(raw_score)))

    # ---- Health state determination ----
    has_any_evidence = any([
        evidence.direct_success_age_hours is not None,
        evidence.seed_age_hours is not None,
        evidence.max_recent_peer_count > 0,
        evidence.dht_sighting_count_12h > 0,
    ])

    if not has_any_evidence:
        health_state = HealthState.UNKNOWN
        health_confidence = 0.0
    elif evidence.direct_success_age_hours is not None and evidence.direct_success_age_hours <= 72:
        # Recent successful direct verification within 72 hours
        health_state = HealthState.VERIFIED
    elif (
        evidence.direct_success_age_hours is not None
        or evidence.seed_age_hours is not None
        or evidence.max_recent_peer_count > 0
    ):
        # Has evidence but no recent direct verification (> 72h or only weak evidence)
        # Check if all evidence is stale (beyond 7 days)
        all_ages = [
            age for age in (
                evidence.direct_success_age_hours,
                evidence.seed_age_hours,
                evidence.peer_evidence_age_hours,
                evidence.latest_dht_sighting_age_hours,
            )
            if age is not None
        ]
        if all_ages and min(all_ages) > 168:  # 7 days
            health_state = HealthState.STALE
        else:
            health_state = HealthState.UNVERIFIED
    else:
        # Only DHT sightings (weak evidence)
        health_state = HealthState.UNVERIFIED

    # ---- Confidence calculation ----
    # Confidence means: "confidence in the health estimate", NOT
    # "likelihood that the torrent is available".
    if health_state == HealthState.UNKNOWN:
        health_confidence = 0.0
    else:
        evidence_ages = [
            age for age in (
                evidence.direct_success_age_hours,
                evidence.seed_age_hours,
                evidence.peer_evidence_age_hours,
                evidence.latest_dht_sighting_age_hours,
            )
            if age is not None
        ]

        most_recent_evidence = min(evidence_ages) if evidence_ages else None

        freshness = (
            decay(most_recent_evidence, 24) if most_recent_evidence is not None else 0.0
        )

        evidence_families_present = sum([
            evidence.direct_success_age_hours is not None,
            evidence.seed_age_hours is not None,
            evidence.max_recent_peer_count > 0,
            evidence.dht_sighting_count_12h > 0,
        ])
        breadth = evidence_families_present / 4.0

        # 60% freshness, 40% breadth
        health_confidence = round(min(1.0, 0.6 * freshness + 0.4 * breadth), 3)

    return HealthScoreResult(
        health_score=health_score,
        health_confidence=health_confidence,
        health_state=health_state,
        algorithm_version=algorithm_version,
        direct_component=direct,
        seed_component=seed,
        peer_component=peer,
        dht_component=dht,
        failure_penalty=failure_penalty,
    )
