"""
Unit tests for deterministic silver rules and invariants.
"""
from src.common.types import ReasonCode
from src.labels.silver_rules import (
    check_empty_payload_fake,
    check_deceptive_double_extension,
    check_password_trap,
    check_homoglyph_spoofing,
    check_critical_sha1_mismatch,
    evaluate_silver_invariants,
)


def test_empty_payload_rule():
    # 1 byte fake claiming to be Windows 10
    assert check_empty_payload_fake(total_size=1, category="Applications") is True
    assert check_empty_payload_fake(total_size=500, category="Movies") is True
    # Legitimate small txt or ebook (Books category allows small)
    assert check_empty_payload_fake(total_size=50000, category="Books & Learning") is False
    # Legitimate movie
    assert check_empty_payload_fake(total_size=1073741824, category="Movies") is False


def test_deceptive_double_extension():
    # .mp4.exe disguised file
    files = [{"path": ["Season 1", "Episode 1.mp4.exe"], "length": 50000}]
    flagged, exts = check_deceptive_double_extension(files=files)
    assert flagged is True
    assert len(exts) == 1

    # Clean file
    files_clean = [{"path": ["Season 1", "Episode 1.mp4"], "length": 50000000}]
    flagged_clean, _ = check_deceptive_double_extension(files=files_clean)
    assert flagged_clean is False


def test_password_trap():
    files = [{"path": ["Instructions", "password.txt"], "length": 42}]
    flagged, traps = check_password_trap(files)
    assert flagged is True
    assert "password.txt" in traps

    files_clean = [{"path": ["README.nfo"], "length": 1024}]
    flagged_clean, _ = check_password_trap(files_clean)
    assert flagged_clean is False


def test_homoglyph_detection():
    # Cyrillic 'а' lookalike inside Latin word
    spoofed = "Micro" + "\u0430" + "soft_Office.iso"
    assert check_homoglyph_spoofing(spoofed) is True
    assert check_homoglyph_spoofing("Microsoft_Office.iso") is False


def test_sha1_mismatch():
    assert check_critical_sha1_mismatch("error: sha1_mismatch for piece 0") is True
    assert check_critical_sha1_mismatch("source_timeout") is False


def test_evaluate_silver_invariants_fake():
    reasons, details = evaluate_silver_invariants(
        name="Windows.11.Pro.x64.iso.exe",
        total_size=1024,
        category="Applications",
        files=[{"path": ["Windows.11.Pro.x64.iso.exe"], "length": 1024}],
        last_error="sha1_mismatch",
    )
    assert ReasonCode.CRITICAL_SHA1_MISMATCH in reasons
    assert ReasonCode.EMPTY_PAYLOAD_FAKE in reasons
    assert ReasonCode.DECEPTIVE_DOUBLE_EXTENSION in reasons
