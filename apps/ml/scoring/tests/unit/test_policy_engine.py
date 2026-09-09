"""
Unit tests for Policy Engine (orthogonality, tiers, deductions, and actions).
"""
from datetime import datetime, timezone, timedelta
from src.common.types import PolicyAction, RiskTier, AvailabilityState, DecisionSource
from src.policy.engine import evaluate_policy, compute_metadata_quality, compute_availability


def test_stale_swarm_never_marked_as_spam():
    """
    CRITICAL INVARIANT TEST:
    An authentic release with 0 peers must retain high integrity and become STALE availability,
    NEVER blocked or suppressed.
    """
    res = evaluate_policy(
        infohash=b"\x01" * 20,
        model_safe_probability=0.98,
        name="Ubuntu.22.04.LTS.Desktop.amd64.iso",
        total_size=3500000000,
        file_count=1,
        files=[{"path": ["ubuntu-22.04-desktop-amd64.iso"], "length": 3500000000}],
        category="Applications",
        seed_confirmed=False,
        swarm_peers=0,
        last_seen=datetime.now(timezone.utc) - timedelta(days=90),
    )

    assert res.integrity_score >= 90
    assert res.risk_tier == RiskTier.SAFE
    assert res.policy_action == PolicyAction.ALLOW
    assert res.availability_state == AvailabilityState.STALE
    assert res.availability_score < 20


def test_malicious_fake_suppression():
    """
    CRITICAL INVARIANT TEST:
    Empty payload / double-ext fakes must trigger SUPPRESS and BLOCKED, regardless of model score.
    """
    res = evaluate_policy(
        infohash=b"\x02" * 20,
        model_safe_probability=0.85, # Model was fooled
        name="Top.Gun.Maverick.2022.1080p.WEBRip.mp4.exe",
        total_size=1024,
        file_count=1,
        files=[{"path": ["Top.Gun.Maverick.2022.1080p.WEBRip.mp4.exe"], "length": 1024}],
        category="Movies",
        seed_confirmed=True,
        swarm_peers=200,
    )

    assert res.integrity_score == 0
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.decision_source == DecisionSource.POLICY


def test_password_trap_deduction():
    # External phishing trap
    res_ext = evaluate_policy(
        infohash=b"\x03" * 20,
        model_safe_probability=0.95,
        name="Photoshop.2024.Crack.Pre-Activated",
        total_size=2000000000,
        file_count=3,
        files=[
            {"path": ["setup.exe"], "length": 1999900000},
            {"path": ["instructions", "unlock_key.url"], "length": 50},
        ],
        category="Applications",
    )

    assert res_ext.integrity_score <= 40 # 95 - 40 (pw trap) - 20 (spam kw) = 35
    assert res_ext.policy_action in (PolicyAction.DOWNRANK, PolicyAction.REVIEW)

    # Harmless local archive note
    res_loc = evaluate_policy(
        infohash=b"\x04" * 20,
        model_safe_probability=1.0,
        name="Jimmy_Awkward_Adventures",
        total_size=2000000000,
        file_count=2,
        files=[
            {"path": ["game.exe"], "length": 1999900000},
            {"path": ["Archive password.txt"], "length": 50},
        ],
        category="Games",
    )
    assert res_loc.integrity_score == 85 # 100 - 15 = 85 (keeps SAFE tier!)
    assert res_loc.risk_tier == RiskTier.SAFE
    assert res_loc.policy_action == PolicyAction.ALLOW
