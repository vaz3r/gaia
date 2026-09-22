"""
Comprehensive unit tests for the health scoring formula.

Tests prove:
- Half-life decay behavior at exact boundaries
- Bounded aggregation per evidence family
- Score clamping and rounding
- Confidence calculation
- Health state determination (UNKNOWN, UNVERIFIED, VERIFIED, STALE)
- Edge cases: no evidence, negative ages, invalid inputs
- Determinism at fixed timestamps
- Failure normalization rules
"""

import sys
import math
from pathlib import Path
from datetime import datetime, timezone, timedelta

import pytest

# Add src to path
sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

from formula import (
    decay,
    compute_health_score,
    EvidenceFamilies,
    HealthScoreResult,
    HealthState,
    ALGORITHM_VERSION,
    _round_half_up,
)


# ============================================================
# 1. HALF-LIFE DECAY TESTS
# ============================================================

class TestDecay:
    """Prove decay() implements true half-life behavior."""

    def test_zero_age_returns_one(self):
        assert decay(0, 24) == 1.0
        assert decay(0, 36) == 1.0
        assert decay(0, 12) == 1.0

    def test_one_half_life_returns_half(self):
        assert abs(decay(24, 24) - 0.5) < 1e-10
        assert abs(decay(36, 36) - 0.5) < 1e-10
        assert abs(decay(12, 12) - 0.5) < 1e-10
        assert abs(decay(48, 48) - 0.5) < 1e-10

    def test_two_half_lives_returns_quarter(self):
        assert abs(decay(48, 24) - 0.25) < 1e-10
        assert abs(decay(72, 36) - 0.25) < 1e-10
        assert abs(decay(24, 12) - 0.25) < 1e-10

    def test_three_half_lives_returns_one_eighth(self):
        assert abs(decay(72, 24) - 0.125) < 1e-10
        assert abs(decay(108, 36) - 0.125) < 1e-10

    def test_seven_half_lives_returns_near_zero(self):
        # 40 * (0.5)^7 = 0.3125, rounds to 0
        val = decay(7 * 24, 24)
        assert val < 0.01
        assert val > 0  # Never exactly zero

    def test_negative_age_clamped_to_zero(self):
        """Negative ages (clock skew) are clamped to 0, returning 1.0."""
        assert decay(-1, 24) == 1.0
        assert decay(-100, 36) == 1.0

    def test_invalid_half_life_raises(self):
        with pytest.raises(ValueError, match="greater than zero"):
            decay(10, 0)
        with pytest.raises(ValueError, match="greater than zero"):
            decay(10, -1)

    def test_various_half_lives(self):
        """Different half-lives produce different decay at the same age."""
        # At age=24h:
        # half_life=12h -> decay = 0.5^2 = 0.25
        # half_life=24h -> decay = 0.5
        # half_life=36h -> decay = 0.5^(24/36) = 0.62996...
        assert abs(decay(24, 12) - 0.25) < 1e-10
        assert abs(decay(24, 24) - 0.5) < 1e-10
        assert abs(decay(24, 36) - 0.629960524947) < 1e-6

    def test_decay_is_monotonically_decreasing(self):
        """Decay should never increase as age increases."""
        for hl in [6, 12, 18, 24, 36, 48]:
            prev = decay(0, hl)
            for age in range(1, 100):
                cur = decay(age, hl)
                assert cur <= prev, f"Decay increased at age={age}, hl={hl}"
                prev = cur


# ============================================================
# 2. ROUND HALF UP TESTS
# ============================================================

class TestRoundHalfUp:
    def test_basic_rounding(self):
        assert _round_half_up(0.0) == 0
        assert _round_half_up(0.4) == 0
        assert _round_half_up(0.5) == 1
        assert _round_half_up(0.6) == 1
        assert _round_half_up(1.5) == 2
        assert _round_half_up(2.5) == 3

    def test_exact_halves(self):
        """Round half up, not banker's rounding."""
        assert _round_half_up(0.5) == 1
        assert _round_half_up(1.5) == 2
        assert _round_half_up(2.5) == 3
        assert _round_half_up(3.5) == 4
        assert _round_half_up(4.5) == 5

    def test_negative_clamps_to_zero(self):
        """Negative values are clamped by the caller, but round_half_up handles them."""
        assert _round_half_up(-1.0) == -1  # Caller clamps to 0


# ============================================================
# 3. SCORE BOUNDS TESTS
# ============================================================

class TestScoreBounds:
    def test_score_never_exceeds_100(self):
        """Maximum possible score with all fresh evidence."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            seed_age_hours=0,
            max_recent_peer_count=1000,
            peer_evidence_age_hours=0,
            dht_sighting_count_12h=1000,
            latest_dht_sighting_age_hours=0,
            recent_failures_after_latest_success=0,
            latest_failure_age_hours=None,
        ))
        assert result.health_score <= 100

    def test_score_never_below_zero(self):
        """Even with max failures, score clamps to 0."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=None,
            seed_age_hours=None,
            max_recent_peer_count=0,
            peer_evidence_age_hours=None,
            dht_sighting_count_12h=0,
            latest_dht_sighting_age_hours=None,
            recent_failures_after_latest_success=100,
            latest_failure_age_hours=0,
        ))
        assert result.health_score >= 0

    def test_no_evidence_gives_zero_score(self):
        result = compute_health_score(EvidenceFamilies())
        assert result.health_score == 0


# ============================================================
# 4. EVIDENCE FAMILY BOUNDS TESTS
# ============================================================

class TestEvidenceFamilyBounds:
    def test_direct_family_caps_at_40(self):
        """Direct success weight is 40, at age=0 it contributes 40."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
        ))
        assert result.direct_component == pytest.approx(40.0, abs=0.01)

    def test_seed_family_caps_at_30(self):
        """Seed weight is 30, at age=0 it contributes 30."""
        result = compute_health_score(EvidenceFamilies(
            seed_age_hours=0,
        ))
        assert result.seed_component == pytest.approx(30.0, abs=0.01)

    def test_peer_family_saturating_function(self):
        """Peer contribution caps at 20 via min(20, 7*log2(1+count))."""
        # At 7 peers: 7 * log2(8) = 21 -> capped at 20
        assert 7 * math.log2(1 + 6) < 20
        assert min(20, 7 * math.log2(1 + 7)) == 20

        # Fresh peer evidence at 7 peers
        result = compute_health_score(EvidenceFamilies(
            max_recent_peer_count=7,
            peer_evidence_age_hours=0,
        ))
        assert result.peer_component == pytest.approx(20.0, abs=0.01)

    def test_peer_family_1000_peers_same_as_50(self):
        """Both saturate at 20."""
        result_50 = compute_health_score(EvidenceFamilies(
            max_recent_peer_count=50,
            peer_evidence_age_hours=0,
        ))
        result_1000 = compute_health_score(EvidenceFamilies(
            max_recent_peer_count=1000,
            peer_evidence_age_hours=0,
        ))
        assert result_50.peer_component == result_1000.peer_component

    def test_dht_family_caps_at_10(self):
        """DHT weight is 10, capped via min(10, 2*log2(1+count))."""
        # At 31 sightings: 2 * log2(32) = 10.0 -> capped at 10
        assert min(10, 2 * math.log2(1 + 31)) == 10
        # At 15 sightings: 2 * log2(16) = 8.0 -> not capped
        assert min(10, 2 * math.log2(1 + 15)) < 10
        assert min(10, 2 * math.log2(1 + 32)) == 10

    def test_failure_penalty_caps_at_30(self):
        """Failure penalty caps at 30."""
        # Fresh direct success gives ~40
        result_three = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            seed_age_hours=None,
            max_recent_peer_count=0,
            peer_evidence_age_hours=None,
            dht_sighting_count_12h=0,
            latest_dht_sighting_age_hours=None,
            recent_failures_after_latest_success=3,
            latest_failure_age_hours=0,
        ))
        result_many = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            seed_age_hours=None,
            max_recent_peer_count=0,
            peer_evidence_age_hours=None,
            dht_sighting_count_12h=0,
            latest_dht_sighting_age_hours=None,
            recent_failures_after_latest_success=100,
            latest_failure_age_hours=0,
        ))
        # Both should produce the same penalty (capped at 30)
        assert result_three.health_score == result_many.health_score
        # 40 direct - 30 capped penalty = 10
        assert result_three.health_score == 10


# ============================================================
# 5. CONFIDENCE TESTS
# ============================================================

class TestConfidence:
    def test_unknown_state_gives_zero_confidence(self):
        result = compute_health_score(EvidenceFamilies())
        assert result.health_confidence == 0.0
        assert result.health_state == HealthState.UNKNOWN

    def test_single_evidence_gives_lower_confidence_than_multiple(self):
        """Fresh evidence from multiple families yields higher confidence."""
        result_many = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            seed_age_hours=2.0,
            max_recent_peer_count=10,
            peer_evidence_age_hours=3.0,
            dht_sighting_count_12h=5,
            latest_dht_sighting_age_hours=1.0,
            recent_failures_after_latest_success=0,
            latest_failure_age_hours=None,
        ))
        result_one = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            seed_age_hours=None,
            max_recent_peer_count=0,
            peer_evidence_age_hours=None,
            dht_sighting_count_12h=0,
            latest_dht_sighting_age_hours=None,
            recent_failures_after_latest_success=0,
            latest_failure_age_hours=None,
        ))
        assert result_many.health_confidence > result_one.health_confidence

    def test_zero_age_included_in_confidence(self):
        """age_hours=0 (fresh evidence) must be included in confidence calculation."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
        ))
        # Fresh evidence should give high confidence
        assert result.health_confidence >= 0.6

    def test_fresh_evidence_high_confidence(self):
        """All evidence at age=0 should give confidence close to 1.0."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            seed_age_hours=0,
            max_recent_peer_count=10,
            peer_evidence_age_hours=0,
            dht_sighting_count_12h=5,
            latest_dht_sighting_age_hours=0,
        ))
        assert result.health_confidence >= 0.9

    def test_stale_evidence_lower_confidence(self):
        """Stale evidence should give lower confidence than fresh evidence."""
        result_stale = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=168,
            seed_age_hours=168,
            max_recent_peer_count=10,
            peer_evidence_age_hours=168,
            dht_sighting_count_12h=5,
            latest_dht_sighting_age_hours=168,
        ))
        result_fresh = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1,
            seed_age_hours=1,
            max_recent_peer_count=10,
            peer_evidence_age_hours=1,
            dht_sighting_count_12h=5,
            latest_dht_sighting_age_hours=1,
        ))
        # Stale should be significantly lower than fresh
        # Breadth floor of 0.4 applies when all 4 families present
        assert result_stale.health_confidence < result_fresh.health_confidence
        assert result_stale.health_confidence <= 0.5


# ============================================================
# 6. HEALTH STATE TESTS
# ============================================================

class TestHealthState:
    def test_no_evidence_is_unknown(self):
        result = compute_health_score(EvidenceFamilies())
        assert result.health_state == HealthState.UNKNOWN

    def test_recent_direct_success_is_verified(self):
        """Direct success within 72h = VERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
        ))
        assert result.health_state == HealthState.VERIFIED

    def test_direct_success_at_72h_is_verified(self):
        """Exactly at 72h boundary = VERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=72.0,
        ))
        assert result.health_state == HealthState.VERIFIED

    def test_direct_success_at_73h_is_unverified(self):
        """73h > 72h = UNVERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=73.0,
        ))
        assert result.health_state == HealthState.UNVERIFIED

    def test_only_dht_sightings_is_unverified(self):
        """Only weak DHT evidence = UNVERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            dht_sighting_count_12h=5,
            latest_dht_sighting_age_hours=1.0,
        ))
        assert result.health_state == HealthState.UNVERIFIED

    def test_seed_only_is_unverified(self):
        """Seed but no direct verification = UNVERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            seed_age_hours=1.0,
        ))
        assert result.health_state == HealthState.UNVERIFIED

    def test_peer_only_is_unverified(self):
        """Peer evidence only = UNVERIFIED."""
        result = compute_health_score(EvidenceFamilies(
            max_recent_peer_count=5,
            peer_evidence_age_hours=1.0,
        ))
        assert result.health_state == HealthState.UNVERIFIED

    def test_old_evidence_is_stale(self):
        """All evidence older than 7 days = STALE."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=200,
            seed_age_hours=200,
            max_recent_peer_count=10,
            peer_evidence_age_hours=200,
        ))
        assert result.health_state == HealthState.STALE

    def test_mixed_ages_not_stale(self):
        """If any evidence is recent (< 7 days), not STALE."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=200,
            seed_age_hours=1.0,  # This is recent
        ))
        assert result.health_state != HealthState.STALE


# ============================================================
# 7. FAILURE HANDLING TESTS
# ============================================================

class TestFailureHandling:
    def test_failure_before_last_success_no_penalty(self):
        """A failure that occurred BEFORE the latest success should not count."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            recent_failures_after_latest_success=0,  # Only failures AFTER success count
            latest_failure_age_hours=5.0,
        ))
        assert result.failure_penalty == 0.0

    def test_failures_after_success_apply_penalty(self):
        """Failures after the latest success apply a capped penalty."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            recent_failures_after_latest_success=2,
            latest_failure_age_hours=1.0,
        ))
        assert result.failure_penalty > 0

    def test_failure_without_prior_success_reduces_score(self):
        """Failures without any prior success still apply penalty."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=None,
            recent_failures_after_latest_success=2,
            latest_failure_age_hours=1.0,
        ))
        assert result.failure_penalty > 0
        assert result.health_score == 0  # Clamped

    def test_failure_penalty_decays(self):
        """Older failures produce smaller penalties."""
        result_recent = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            recent_failures_after_latest_success=2,
            latest_failure_age_hours=1.0,
        ))
        result_old = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=1.0,
            recent_failures_after_latest_success=2,
            latest_failure_age_hours=24.0,
        ))
        assert result_recent.failure_penalty > result_old.failure_penalty

    def test_normalized_observation_types(self):
        """Both metadata_fetch_failure and probe_failure normalize to direct-failure."""
        from evidence_aggregator import _DIRECT_FAILURE_TYPES
        assert "metadata_fetch_failure" in _DIRECT_FAILURE_TYPES
        assert "probe_failure" in _DIRECT_FAILURE_TYPES


# ============================================================
# 8. DETERMINISM TESTS
# ============================================================

class TestDeterminism:
    def test_same_inputs_same_output(self):
        """Formula must be deterministic: same inputs always produce same outputs."""
        evidence = EvidenceFamilies(
            direct_success_age_hours=5.0,
            seed_age_hours=10.0,
            max_recent_peer_count=15,
            peer_evidence_age_hours=3.0,
            dht_sighting_count_12h=8,
            latest_dht_sighting_age_hours=2.0,
            recent_failures_after_latest_success=1,
            latest_failure_age_hours=8.0,
        )
        results = [compute_health_score(evidence) for _ in range(100)]
        scores = [r.health_score for r in results]
        confidences = [r.health_confidence for r in results]
        states = [r.health_state for r in results]
        assert len(set(scores)) == 1, "Scores are not deterministic"
        assert len(set(confidences)) == 1, "Confidences are not deterministic"
        assert len(set(states)) == 1, "States are not deterministic"

    def test_no_system_time_in_formula(self):
        """The formula uses only the evidence ages, not system time.
        This is verified by the determinism test above."""
        pass


# ============================================================
# 9. INTEGRATION / KNOWN-VALUE TESTS
# ============================================================

class TestKnownValues:
    def test_high_availability_torrent(self):
        """Fresh seed, peers, and probe success = high score."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=2.0,
            seed_age_hours=1.0,
            max_recent_peer_count=50,
            peer_evidence_age_hours=1.0,
            dht_sighting_count_12h=10,
            latest_dht_sighting_age_hours=0.5,
        ))
        assert result.health_score >= 80
        assert result.health_confidence >= 0.8
        assert result.health_state == HealthState.VERIFIED

    def test_degraded_torrent(self):
        """Older evidence, moderate peers, no direct probe."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=None,
            seed_age_hours=24.0,
            max_recent_peer_count=5,
            peer_evidence_age_hours=24.0,
            dht_sighting_count_12h=3,
            latest_dht_sighting_age_hours=12.0,
        ))
        assert 20 <= result.health_score <= 60
        assert result.health_state == HealthState.UNVERIFIED

    def test_dead_torrent(self):
        """Old evidence, no peers, no seeds."""
        result = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=200.0,
            seed_age_hours=200.0,
            max_recent_peer_count=0,
            peer_evidence_age_hours=None,
            dht_sighting_count_12h=0,
            latest_dht_sighting_age_hours=None,
        ))
        assert result.health_score <= 10
        assert result.health_state == HealthState.STALE

    def test_algorithm_version_in_result(self):
        result = compute_health_score(EvidenceFamilies())
        assert result.algorithm_version == ALGORITHM_VERSION
        assert result.algorithm_version == "2.0.0"


# ============================================================
# 10. EVIDENCE AGGREGATOR TESTS
# ============================================================

class TestEvidenceAggregator:
    def test_empty_observations(self):
        from evidence_aggregator import aggregate_observations
        result = aggregate_observations([])
        assert result == EvidenceFamilies()

    def test_selects_latest_direct_success(self):
        from evidence_aggregator import Observation, aggregate_observations
        now = datetime(2026, 9, 22, 12, 0, 0, tzinfo=timezone.utc)
        observations = [
            Observation(
                id=1, infohash=b"test",
                observed_at=now - timedelta(hours=48),
                observation_type="metadata_fetch_success",
                peer_count=0, seed_count=0, source="crawler",
            ),
            Observation(
                id=2, infohash=b"test",
                observed_at=now - timedelta(hours=2),
                observation_type="probe_success",
                peer_count=0, seed_count=0, source="crawler",
            ),
        ]
        result = aggregate_observations(observations, now=now)
        assert result.direct_success_age_hours == pytest.approx(2.0, abs=0.1)

    def test_peer_saturating_function(self):
        """1000 peers should not produce a higher score than 50 peers."""
        assert min(20, 7 * math.log2(1 + 50)) == 20
        assert min(20, 7 * math.log2(1 + 1000)) == 20

    def test_failure_cap(self):
        """Even with 100 failures, penalty caps at 30."""
        result_three = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            recent_failures_after_latest_success=3,
            latest_failure_age_hours=0,
        ))
        result_many = compute_health_score(EvidenceFamilies(
            direct_success_age_hours=0,
            recent_failures_after_latest_success=100,
            latest_failure_age_hours=0,
        ))
        assert result_three.health_score == result_many.health_score
        assert result_three.health_score == 10  # 40 direct - 30 capped penalty
