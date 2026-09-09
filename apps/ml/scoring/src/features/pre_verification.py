"""
Point-in-time feature extraction for Pre-Verification prioritization.
Guarantees zero future temporal leakage.
"""
import math
from typing import Dict, Any, List, Optional
from datetime import datetime, timezone


def extract_pre_verification_features(
    first_seen: Optional[datetime],
    last_seen: Optional[datetime],
    source_counts: Optional[Dict[str, Any]],
    total_seen: int,
    prior_attempts: int = 0,
    last_attempt_at: Optional[datetime] = None,
    decision_time: Optional[datetime] = None,
) -> Dict[str, float]:
    """
    Extracts features strictly as of decision_time t.
    """
    if decision_time is None:
        decision_time = datetime.now(timezone.utc)

    if first_seen and first_seen.tzinfo is None:
        first_seen = first_seen.replace(tzinfo=timezone.utc)
    if last_seen and last_seen.tzinfo is None:
        last_seen = last_seen.replace(tzinfo=timezone.utc)
    if last_attempt_at and last_attempt_at.tzinfo is None:
        last_attempt_at = last_attempt_at.replace(tzinfo=timezone.utc)

    # 1. Sighting timing
    sec_since_first = max(0.0, (decision_time - first_seen).total_seconds()) if first_seen else 0.0
    sec_since_last = max(0.0, (decision_time - last_seen).total_seconds()) if last_seen else 0.0

    # 2. Source counts (get_peers vs announce_peer)
    counts = source_counts or {}
    get_peers = counts.get("get_peers", 0)
    announce_peers = counts.get("announce_peer", 0)
    total_sources = get_peers + announce_peers

    announce_ratio = announce_peers / total_sources if total_sources > 0 else 0.0
    log_sightings = math.log10(1.0 + max(0, total_seen))

    # 3. Interarrival estimation
    sighting_span = max(1.0, sec_since_first - sec_since_last)
    interarrival_mean = sighting_span / max(1, total_seen)

    # 4. Attempt history
    sec_since_last_attempt = max(0.0, (decision_time - last_attempt_at).total_seconds()) if last_attempt_at else -1.0

    # 5. Temporal / diurnal features
    hour = decision_time.hour
    day = decision_time.weekday()

    return {
        "seconds_since_first_sighting": sec_since_first,
        "seconds_since_last_sighting": sec_since_last,
        "log_seconds_since_last": math.log10(1.0 + sec_since_last),
        "announce_to_get_peers_ratio": announce_ratio,
        "log_sighting_count": log_sightings,
        "sighting_interarrival_mean": interarrival_mean,
        "prior_attempt_count": float(prior_attempts),
        "seconds_since_last_attempt": sec_since_last_attempt,
        "hour_of_day_sin": math.sin(2.0 * math.pi * hour / 24.0),
        "hour_of_day_cos": math.cos(2.0 * math.pi * hour / 24.0),
        "day_of_week": float(day),
    }
