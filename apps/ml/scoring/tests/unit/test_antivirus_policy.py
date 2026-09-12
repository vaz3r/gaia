import pytest
from src.common.types import ReasonCode, RiskTier, PolicyAction
from src.labels.silver_rules import (
    evaluate_silver_invariants,
    check_executable_in_media,
    check_rtlo_spoofing,
    check_standalone_script_exploit,
    check_implausible_software_size,
    check_deceptive_double_extension,
)
from src.policy.engine import evaluate_policy
from src.features.integrity import extract_integrity_features


def test_media_single_executable_blocked():
    reasons, details = evaluate_silver_invariants(
        name="Moana.2.2026.1080p.WEBRip.x264.exe",
        total_size=1500000000,
        category="Movies",
        files=[{"path": "Moana.2.2026.1080p.WEBRip.x264.exe", "length": 1500000000}],
    )
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.99,
        name="Moana.2.2026.1080p.WEBRip.x264.exe",
        total_size=1500000000,
        file_count=1,
        files=[{"path": "Moana.2.2026.1080p.WEBRip.x264.exe", "length": 1500000000}],
        category="Movies",
    )
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.integrity_score == 0
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM.value in res.reason_codes


def test_media_screensaver_malware_blocked():
    reasons, details = evaluate_silver_invariants(
        name="House of the Dragon S03E06 1080p HEVC x265-MeGusta.scr",
        total_size=1163449856,
        category="Television",
        files=[{"path": "House of the Dragon S03E06 1080p HEVC x265-MeGusta.scr", "length": 1163449856}],
    )
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.95,
        name="House of the Dragon S03E06 1080p HEVC x265-MeGusta.scr",
        total_size=1163449856,
        file_count=1,
        files=[{"path": "House of the Dragon S03E06 1080p HEVC x265-MeGusta.scr", "length": 1163449856}],
        category="Television",
    )
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.integrity_score == 0


def test_rtlo_spoofing_attack_blocked():
    exploit_title = "Avengers Doomsday UHD BDRemux 2160p  4K  HDR  Dolby Vision  D, P  Refl‮vkm.exe"
    reasons, details = evaluate_silver_invariants(
        name=exploit_title,
        total_size=45000000000,
        category="Movies",
        files=[{"path": exploit_title, "length": 45000000000}],
    )
    assert ReasonCode.RTLO_CHAR_SPOOFING in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.99,
        name=exploit_title,
        total_size=45000000000,
        file_count=1,
        files=[{"path": exploit_title, "length": 45000000000}],
        category="Movies",
    )
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.integrity_score == 0
    assert ReasonCode.RTLO_CHAR_SPOOFING.value in res.reason_codes


def test_multi_file_media_executable_payload_blocked():
    folder_name = "The Road (2009) 1080p.BluRay.x264.Full 553MB"
    files = [
        {"path": "Downloaded from THEPIRATEBAY.ORG.txt", "length": 120},
        {"path": "The Road (2009) 1080p.BluRay.x264.Full 553MB.exe", "length": 580000000},
    ]
    reasons, details = evaluate_silver_invariants(
        name=folder_name,
        total_size=580000120,
        category="Movies",
        files=files,
    )
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.90,
        name=folder_name,
        total_size=580000120,
        file_count=2,
        files=files,
        category="Movies",
    )
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.integrity_score == 0


def test_standalone_script_exploit_blocked():
    reasons, details = evaluate_silver_invariants(
        name="Keygen_Activator.scr",
        total_size=250000,
        category="Applications",
        files=[{"path": "Keygen_Activator.scr", "length": 250000}],
    )
    assert ReasonCode.STANDALONE_SCRIPT_EXPLOIT in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.85,
        name="Keygen_Activator.scr",
        total_size=250000,
        file_count=1,
        files=[{"path": "Keygen_Activator.scr", "length": 250000}],
        category="Applications",
    )
    assert res.policy_action == PolicyAction.SUPPRESS
    assert res.risk_tier == RiskTier.BLOCKED
    assert res.integrity_score == 0


def test_space_padded_double_extension_blocked():
    name = "Avatar.2.1080p.WEB-DL.mp4                 .exe"
    reasons, details = evaluate_silver_invariants(
        name=name,
        total_size=2000000000,
        category="Movies",
        files=[{"path": name, "length": 2000000000}],
    )
    assert ReasonCode.DECEPTIVE_DOUBLE_EXTENSION in reasons
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM in reasons


def test_legitimate_games_not_blocked():
    game_files = [
        {"path": "Cyberpunk 2077/setup.exe", "length": 50000000},
        {"path": "Cyberpunk 2077/data1.bin", "length": 30000000000},
        {"path": "Cyberpunk 2077/data2.bin", "length": 35000000000},
    ]
    reasons, details = evaluate_silver_invariants(
        name="Cyberpunk 2077 v2.1-FitGirl",
        total_size=65050000000,
        category="Games",
        files=game_files,
    )
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM not in reasons
    assert ReasonCode.STANDALONE_SCRIPT_EXPLOIT not in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.98,
        name="Cyberpunk 2077 v2.1-FitGirl",
        total_size=65050000000,
        file_count=3,
        files=game_files,
        category="Games",
    )
    assert res.policy_action == PolicyAction.ALLOW
    assert res.risk_tier == RiskTier.SAFE
    assert res.integrity_score >= 80


def test_legitimate_applications_not_blocked():
    app_files = [
        {"path": "VSCodeUserSetup-x64-1.93.0.exe", "length": 95000000},
    ]
    reasons, details = evaluate_silver_invariants(
        name="VSCodeUserSetup-x64-1.93.0.exe",
        total_size=95000000,
        category="Applications",
        files=app_files,
    )
    assert ReasonCode.EXECUTABLE_IN_MEDIA_SWARM not in reasons
    assert ReasonCode.STANDALONE_SCRIPT_EXPLOIT not in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.95,
        name="VSCodeUserSetup-x64-1.93.0.exe",
        total_size=95000000,
        file_count=1,
        files=app_files,
        category="Applications",
    )
    assert res.policy_action == PolicyAction.ALLOW
    assert res.risk_tier == RiskTier.SAFE


def test_implausible_software_size_penalized():
    reasons, details = evaluate_silver_invariants(
        name="Adobe Photoshop 2026 Full Activated.exe",
        total_size=1200000,
        category="Applications",
        files=[{"path": "Adobe Photoshop 2026 Full Activated.exe", "length": 1200000}],
    )
    assert ReasonCode.IMPLAUSIBLE_SOFTWARE_PAYLOAD in reasons

    res = evaluate_policy(
        infohash=b"" * 20,
        model_safe_probability=0.70,
        name="Adobe Photoshop 2026 Full Activated.exe",
        total_size=1200000,
        file_count=1,
        files=[{"path": "Adobe Photoshop 2026 Full Activated.exe", "length": 1200000}],
        category="Applications",
    )
    assert res.policy_action != PolicyAction.ALLOW
    assert res.integrity_score <= 20


def test_feature_extraction_antivirus_signals():
    feats_media_malware = extract_integrity_features(
        name="Moana.2.2026.exe",
        total_size=1500000000,
        file_count=1,
        category="Movies",
    )
    assert feats_media_malware["category_is_media"] == 1.0
    assert feats_media_malware["category_is_software"] == 0.0
    assert feats_media_malware["media_executable_mismatch"] == 1.0
    assert feats_media_malware["media_standalone_executable"] == 1.0

    feats_legit_game = extract_integrity_features(
        name="Cyberpunk 2077 Setup.exe",
        total_size=65000000000,
        file_count=1,
        category="Games",
    )
    assert feats_legit_game["category_is_media"] == 0.0
    assert feats_legit_game["category_is_software"] == 1.0
    assert feats_legit_game["media_executable_mismatch"] == 0.0
    assert feats_legit_game["media_standalone_executable"] == 0.0
    assert feats_legit_game["software_implausible_size"] == 0.0

    feats_fake_app = extract_integrity_features(
        name="Adobe Photoshop 2026 Full.exe",
        total_size=1200000,
        file_count=1,
        category="Applications",
    )
    assert feats_fake_app["category_is_software"] == 1.0
    assert feats_fake_app["software_implausible_size"] == 1.0
