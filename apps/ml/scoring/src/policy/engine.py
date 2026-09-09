"""
Deterministic Policy Engine for Torrent Scoring.
Combines calibrated model probabilities, deterministic safety deductions,
quality metrics, and operational availability states.
"""
import math
import re
import json
from datetime import datetime, timezone
from typing import Dict, List, Any, Optional, Tuple

from src.common.types import (
    PolicyAction,
    RiskTier,
    AvailabilityState,
    DecisionSource,
    ReasonCode,
    ScoringResult,
)
from src.labels.silver_rules import evaluate_silver_invariants

# Load default policy weights
POLICY_VERSION = "v1.0.0"

SPAM_KEYWORDS = {
    "crack", "keygen", "serial", "patch", "activation", "unlock",
    "free download", "full version", "registration code"
}

JUNK_EXTENSIONS = {".nfo", ".txt", ".url", ".website", ".lnk", ".diz", ".ion"}


def compute_metadata_quality(
    name: Optional[str] = None,
    total_size: int = 0,
    file_count: int = 0,
    files: Optional[List[Dict[str, Any]]] = None,
    category: Optional[str] = None,
) -> Tuple[int, Dict[str, int]]:
    """
    V1 Explicit Deterministic Quality formulation: 100 - sum(penalties).
    """
    penalties: Dict[str, int] = {}
    name_clean = (name or "").strip()

    # 1. Title quality check: hex hash or uninformative name
    if re.match(r"^[0-9a-fA-F]{32,64}$", name_clean) or len(name_clean) < 4:
        penalties["missing_clean_title"] = 25
    elif re.search(r"\b(1080p|720p|2160p|4k|web-dl|bluray|x264|x265|hevc|repack|flac|mp3)\b", name_clean, re.IGNORECASE):
        # Good release structure
        pass

    # 2. File list and payload dominance
    if files and len(files) > 0 and total_size > 0:
        file_sizes = [int(f.get("length") or f.get("size") or 0) for f in files if isinstance(f, dict)]
        max_file_size = max(file_sizes) if file_sizes else 0
        primary_share = max_file_size / total_size

        cat_lower = (category or "").lower()
        if any(c in cat_lower for c in ("movie", "film", "television", "tv", "video", "anime")) and file_count <= 10:
            if primary_share < 0.60:
                penalties["low_primary_payload_share"] = 20

        # 3. Path depth
        max_depth = 0
        junk_files = 0
        extensions = set()

        for f in files:
            p_val = f.get("path") if isinstance(f, dict) else ""
            if isinstance(p_val, list):
                path = "/".join(str(seg) for seg in p_val)
            else:
                path = str(p_val or (f.get("name") if isinstance(f, dict) else ""))

            depth = path.count("/") + path.count("\\")
            if depth > max_depth:
                max_depth = depth
            
            ext = "." + path.rsplit(".", 1)[-1].lower() if "." in path else ""
            if ext:
                extensions.add(ext)
            if ext in JUNK_EXTENSIONS:
                junk_files += 1

        if max_depth > 6:
            penalties["excessive_path_depth"] = 10

        junk_ratio = junk_files / len(files)
        if junk_ratio > 0.35 and len(files) > 5:
            penalties["high_junk_file_ratio"] = 15

        if len(extensions) > 8:
            penalties["unusual_extension_diversity"] = 10

    total_penalty = sum(penalties.values())
    quality_score = max(0, min(100, 100 - total_penalty))
    return quality_score, penalties


def compute_availability(
    seed_confirmed: bool = False,
    swarm_peers: Optional[int] = 0,
    last_seen: Optional[datetime] = None,
    now: Optional[datetime] = None,
) -> Tuple[int, AvailabilityState]:
    """
    V1 Explicit Deterministic Availability formulation: time-decayed operational score.
    """
    if now is None:
        now = datetime.now(timezone.utc)

    # 1. Confirmed seed component (up to 50 pts)
    s_seed = 50 if seed_confirmed else 0

    # 2. Peer count log component (up to 30 pts)
    peers = max(0, swarm_peers or 0)
    s_peer = min(30, int(6.0 * math.log2(1.0 + peers))) if peers > 0 else 0

    # 3. Sighting recency component (up to 20 pts)
    s_recency = 0
    if last_seen is not None:
        if last_seen.tzinfo is None:
            last_seen = last_seen.replace(tzinfo=timezone.utc)
        delta_days = max(0.0, (now - last_seen).total_seconds() / 86400.0)
        s_recency = int(20.0 * math.exp(-delta_days / 7.0))
    else:
        # Never sighted
        s_recency = 0

    availability_score = min(100, s_seed + s_peer + s_recency)

    if availability_score >= 60:
        state = AvailabilityState.ACTIVE
    elif availability_score >= 25:
        state = AvailabilityState.DEGRADED
    elif availability_score > 0 or last_seen is not None:
        state = AvailabilityState.STALE
    else:
        state = AvailabilityState.UNKNOWN

    return availability_score, state


def evaluate_policy(
    infohash: bytes,
    model_safe_probability: float,
    name: Optional[str] = None,
    total_size: int = 0,
    file_count: int = 0,
    files: Optional[List[Dict[str, Any]]] = None,
    category: Optional[str] = None,
    seed_confirmed: bool = False,
    swarm_peers: Optional[int] = 0,
    last_seen: Optional[datetime] = None,
    last_error: Optional[str] = None,
    score_model_version: str = "v1.0.0",
) -> ScoringResult:
    """
    Evaluates policy deductions, invariant rules, and generates ScoringResult.
    """
    # 1. Evaluate deterministic silver invariants
    triggered_reasons, details = evaluate_silver_invariants(
        name=name,
        total_size=total_size,
        category=category,
        files=files,
        last_error=last_error,
    )

    reason_code_strings = [r.value for r in triggered_reasons]

    # 2. Calculate Quality & Availability sub-scores
    quality_score, quality_deductions = compute_metadata_quality(
        name=name,
        total_size=total_size,
        file_count=file_count,
        files=files,
        category=category,
    )

    avail_score, avail_state = compute_availability(
        seed_confirmed=seed_confirmed,
        swarm_peers=swarm_peers,
        last_seen=last_seen,
    )

    # 3. Check Critical Invariants (Forces SUPPRESS & BLOCKED)
    critical_invariants = {
        ReasonCode.CRITICAL_SHA1_MISMATCH,
        ReasonCode.EMPTY_PAYLOAD_FAKE,
        ReasonCode.DECEPTIVE_DOUBLE_EXTENSION,
    }
    has_critical = any(r in critical_invariants for r in triggered_reasons)

    if has_critical:
        policy_integrity = 0
        integrity_score = 0
        risk_tier = RiskTier.BLOCKED
        policy_action = PolicyAction.SUPPRESS
        decision_source = DecisionSource.POLICY
    else:
        # Start from calibrated model probability
        baseline = int(round(model_safe_probability * 100))
        deductions = 0

        if ReasonCode.PASSWORD_TRAP_SUSPECTED in triggered_reasons:
            deductions += 40
        elif ReasonCode.LOCAL_PASSWORD_NOTE in triggered_reasons:
            deductions += 15

        if ReasonCode.HOMOGLYPH_PATH_SPOOFING in triggered_reasons:
            deductions += 50

        # Check spam keywords in title
        name_lower = (name or "").lower()
        if any(k in name_lower for k in SPAM_KEYWORDS):
            reason_code_strings.append(ReasonCode.SUSPICIOUS_SPAM_KEYWORDS.value)
            deductions += 20

        policy_integrity = max(0, min(100, baseline - deductions))
        integrity_score = policy_integrity

        # Assign Risk Tier
        if policy_integrity >= 80:
            risk_tier = RiskTier.SAFE
        elif policy_integrity >= 50:
            risk_tier = RiskTier.REVIEW
        elif policy_integrity >= 20:
            risk_tier = RiskTier.SUSPICIOUS
        else:
            risk_tier = RiskTier.BLOCKED

        # Assign Policy Action
        if policy_integrity >= 70:
            if 0.40 <= model_safe_probability <= 0.60:
                policy_action = PolicyAction.REVIEW
            else:
                policy_action = PolicyAction.ALLOW
        elif policy_integrity >= 40:
            policy_action = PolicyAction.DOWNRANK
        else:
            # Low score alone gets REVIEW or DOWNRANK, NEVER SUPPRESS without critical invariant or verified threshold
            policy_action = PolicyAction.REVIEW

        decision_source = DecisionSource.MODEL_AND_POLICY if deductions > 0 else DecisionSource.MODEL

    return ScoringResult(
        infohash=infohash,
        model_safe_probability=model_safe_probability,
        policy_integrity_score=policy_integrity,
        integrity_score=integrity_score,
        metadata_quality_score=quality_score,
        availability_score=avail_score,
        risk_tier=risk_tier,
        availability_state=avail_state,
        policy_action=policy_action,
        decision_source=decision_source,
        policy_version=POLICY_VERSION,
        score_model_version=score_model_version,
        reason_codes=reason_code_strings,
        quality_deductions=quality_deductions,
        scored_at=datetime.now(timezone.utc),
    )
