"""
Evidence aggregator: converts raw observations into bounded evidence families.

This module takes a list of observations for a single infohash and produces
an EvidenceFamilies instance with one aggregated value per family, applying
saturating functions and selecting the latest relevant event.
"""

from __future__ import annotations

import logging
import math
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import List, Optional

try:
    from .config import DHT_SIGHTING_WINDOW_HOURS, FAILURE_CAP, FAILURE_LOOKBACK_HOURS
    from .formula import EvidenceFamilies
except ImportError:
    from config import DHT_SIGHTING_WINDOW_HOURS, FAILURE_CAP, FAILURE_LOOKBACK_HOURS
    from formula import EvidenceFamilies

logger = logging.getLogger(__name__)


@dataclass
class Observation:
    """Raw observation row from torrent_availability_observations or legacy adapter."""

    id: int
    infohash: bytes
    observed_at: datetime
    observation_type: str
    peer_count: int
    seed_count: int
    source: str
    latency_ms: Optional[int] = None
    failure_reason: Optional[str] = None


def _hours_since(observed_at: datetime, now: datetime) -> float:
    """Calculate hours between observed_at and now. Handles timezone-aware datetimes."""
    if observed_at.tzinfo is None:
        observed_at = observed_at.replace(tzinfo=timezone.utc)
    if now.tzinfo is None:
        now = now.replace(tzinfo=timezone.utc)
    delta = now - observed_at
    return max(0.0, delta.total_seconds() / 3600.0)


# Normalization: map raw observation types to scoring families
_DIRECT_SUCCESS_TYPES = {"metadata_fetch_success", "probe_success"}
_DIRECT_FAILURE_TYPES = {"metadata_fetch_failure", "probe_failure"}
_SEED_TYPES = {"seed_confirmed"}
_PEER_TYPES = {"peer_seen"}
_DHT_TYPES = {"dht_sighting"}


def aggregate_observations(
    observations: List[Observation],
    now: Optional[datetime] = None,
) -> EvidenceFamilies:
    """Aggregate raw observations into bounded evidence families.

    For each evidence family, selects the single most relevant value:
    - Direct success: latest successful event only
    - Seed: latest confirmed-seed event only
    - Peer: max peer count from the most recent peer observation, with saturating log
    - DHT: count in the 12h window, latest sighting age
    - Failures: count after latest success, capped, with decay

    Args:
        observations: List of Observation objects for a single infohash.
        now: Current time (for age calculations). Defaults to UTC now.

    Returns:
        EvidenceFamilies with one aggregated value per family.
    """
    if not observations:
        return EvidenceFamilies()

    if now is None:
        now = datetime.now(timezone.utc)

    # Sort by observed_at descending (most recent first)
    sorted_obs = sorted(observations, key=lambda o: o.observed_at, reverse=True)

    # --- Family 1: Direct probe/metadata success (latest only) ---
    direct_success_age = None
    latest_success_time = None
    for obs in sorted_obs:
        if obs.observation_type in _DIRECT_SUCCESS_TYPES:
            latest_success_time = obs.observed_at
            direct_success_age = _hours_since(obs.observed_at, now)
            break

    # --- Family 2: Confirmed seed (latest only) ---
    seed_age = None
    for obs in sorted_obs:
        if obs.observation_type in _SEED_TYPES:
            seed_age = _hours_since(obs.observed_at, now)
            break

    # --- Family 3: Peer count (most recent peer observation, saturating log) ---
    peer_count = 0
    peer_age = None
    for obs in sorted_obs:
        if obs.observation_type in _PEER_TYPES:
            peer_count = obs.peer_count
            peer_age = _hours_since(obs.observed_at, now)
            break

    # --- Family 4: DHT sightings (count in 12h window, latest age) ---
    now_utc = now if now.tzinfo else now.replace(tzinfo=timezone.utc)
    dht_count_12h = 0
    dht_latest_age = None
    for obs in sorted_obs:
        if obs.observation_type not in _DHT_TYPES:
            continue
        age = _hours_since(obs.observed_at, now)
        if age <= DHT_SIGHTING_WINDOW_HOURS:
            dht_count_12h += 1
        if dht_latest_age is None:
            dht_latest_age = age

    # --- Failure penalty: failures after latest success, capped ---
    recent_failures = 0
    latest_failure_age = None

    # Find failures that occurred AFTER the latest success, within lookback window
    if latest_success_time is not None:
        for obs in sorted_obs:
            if obs.observation_type in _DIRECT_FAILURE_TYPES:
                if obs.observed_at > latest_success_time:
                    age = _hours_since(obs.observed_at, now)
                    if age <= FAILURE_LOOKBACK_HOURS:
                        recent_failures += 1
                        if latest_failure_age is None:
                            latest_failure_age = age
                        if recent_failures >= FAILURE_CAP:
                            break
    else:
        # No prior success: failures reduce confidence but don't produce
        # an overconfident "dead" result. Only count recent failures.
        for obs in sorted_obs:
            if obs.observation_type in _DIRECT_FAILURE_TYPES:
                age = _hours_since(obs.observed_at, now)
                if age <= FAILURE_LOOKBACK_HOURS:
                    recent_failures += 1
                    if latest_failure_age is None:
                        latest_failure_age = age
                    if recent_failures >= FAILURE_CAP:
                        break

    return EvidenceFamilies(
        direct_success_age_hours=direct_success_age,
        seed_age_hours=seed_age,
        max_recent_peer_count=peer_count,
        peer_evidence_age_hours=peer_age,
        dht_sighting_count_12h=dht_count_12h,
        latest_dht_sighting_age_hours=dht_latest_age,
        recent_failures_after_latest_success=min(recent_failures, FAILURE_CAP),
        latest_failure_age_hours=latest_failure_age,
    )
