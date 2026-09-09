"""
Core types, enums, and data structures for Torrent Quality, Trust & Availability Scoring.
"""
from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, List, Optional, Any
from datetime import datetime


class PolicyAction(str, Enum):
    ALLOW = "ALLOW"
    DOWNRANK = "DOWNRANK"
    REVIEW = "REVIEW"
    SUPPRESS = "SUPPRESS"


class RiskTier(str, Enum):
    SAFE = "SAFE"
    REVIEW = "REVIEW"
    SUSPICIOUS = "SUSPICIOUS"
    BLOCKED = "BLOCKED"


class AvailabilityState(str, Enum):
    ACTIVE = "ACTIVE"
    DEGRADED = "DEGRADED"
    UNKNOWN = "UNKNOWN"
    STALE = "STALE"


class DecisionSource(str, Enum):
    MODEL = "MODEL"
    POLICY = "POLICY"
    MODEL_AND_POLICY = "MODEL_AND_POLICY"
    MANUAL = "MANUAL"


class ScoreStatus(str, Enum):
    VALID = "VALID"
    DEPRECATED = "DEPRECATED"
    SUPERSEDED = "SUPERSEDED"
    INVALIDATED = "INVALIDATED"


class ReasonCode(str, Enum):
    # Critical Deterministic Invariants (triggers SUPPRESS / BLOCKED)
    CRITICAL_SHA1_MISMATCH = "CRITICAL_SHA1_MISMATCH"
    EMPTY_PAYLOAD_FAKE = "EMPTY_PAYLOAD_FAKE"
    DECEPTIVE_DOUBLE_EXTENSION = "DECEPTIVE_DOUBLE_EXTENSION"
    EXECUTABLE_IN_MEDIA_SWARM = "EXECUTABLE_IN_MEDIA_SWARM"
    
    # Suspicious Signals (deductions / DOWNRANK / REVIEW)
    PASSWORD_TRAP_SUSPECTED = "PASSWORD_TRAP_SUSPECTED"
    HOMOGLYPH_PATH_SPOOFING = "HOMOGLYPH_PATH_SPOOFING"
    IMPLAUSIBLE_CATEGORY_SIZE = "IMPLAUSIBLE_CATEGORY_SIZE"
    SUSPICIOUS_SPAM_KEYWORDS = "SUSPICIOUS_SPAM_KEYWORDS"
    HIGH_JUNK_FILE_RATIO = "HIGH_JUNK_FILE_RATIO"
    EXCESSIVE_PATH_DEPTH = "EXCESSIVE_PATH_DEPTH"
    ANOMALOUS_PIECE_SIZE = "ANOMALOUS_PIECE_SIZE"
    UNUSUAL_EXTENSIONS = "UNUSUAL_EXTENSIONS"
    LOW_PRIMARY_PAYLOAD_SHARE = "LOW_PRIMARY_PAYLOAD_SHARE"
    
    # Clean Signals
    CLEAN_VERIFIED_RELEASE = "CLEAN_VERIFIED_RELEASE"
    ESTABLISHED_SCENE_RELEASE = "ESTABLISHED_SCENE_RELEASE"
    HIGH_PAYLOAD_INTEGRITY = "HIGH_PAYLOAD_INTEGRITY"
    
    # Availability Signals
    HEALTHY_CONFIRMED_SEED = "HEALTHY_CONFIRMED_SEED"
    NO_ACTIVE_PEERS_REPORTED = "NO_ACTIVE_PEERS_REPORTED"
    LONG_ABSENT_SWARM = "LONG_ABSENT_SWARM"


@dataclass
class ScoringResult:
    infohash: bytes
    model_safe_probability: float           # Calibrated P(safe) in [0.0, 1.0]
    policy_integrity_score: int             # 0 to 100 after policy deductions
    integrity_score: int                    # Public display composite integrity score (0 to 100)
    metadata_quality_score: int             # 0 to 100 (deterministic quality)
    availability_score: int                 # 0 to 100 (operational swarm health)
    risk_tier: RiskTier                     # SAFE, REVIEW, SUSPICIOUS, BLOCKED
    availability_state: AvailabilityState   # ACTIVE, DEGRADED, UNKNOWN, STALE
    policy_action: PolicyAction             # ALLOW, DOWNRANK, REVIEW, SUPPRESS
    decision_source: DecisionSource         # MODEL, POLICY, MODEL_AND_POLICY, MANUAL
    policy_version: str
    score_model_version: str
    reason_codes: List[str] = field(default_factory=list)
    quality_deductions: Dict[str, int] = field(default_factory=dict)
    scored_at: datetime = field(default_factory=datetime.utcnow)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "infohash": self.infohash.hex() if isinstance(self.infohash, (bytes, bytearray)) else str(self.infohash),
            "model_safe_probability": round(self.model_safe_probability, 4),
            "policy_integrity_score": self.policy_integrity_score,
            "integrity_score": self.integrity_score,
            "metadata_quality_score": self.metadata_quality_score,
            "availability_score": self.availability_score,
            "risk_tier": self.risk_tier.value,
            "availability_state": self.availability_state.value,
            "policy_action": self.policy_action.value,
            "decision_source": self.decision_source.value,
            "policy_version": self.policy_version,
            "score_model_version": self.score_model_version,
            "reason_codes": self.reason_codes,
            "quality_deductions": self.quality_deductions,
            "scored_at": self.scored_at.isoformat(),
        }
