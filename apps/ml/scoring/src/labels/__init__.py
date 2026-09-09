"""
Label generation and silver rule evaluation.
"""
from src.labels.silver_rules import (
    evaluate_silver_invariants,
    check_critical_sha1_mismatch,
    check_empty_payload_fake,
    check_deceptive_double_extension,
    check_password_trap,
    check_homoglyph_spoofing,
)

__all__ = [
    "evaluate_silver_invariants",
    "check_critical_sha1_mismatch",
    "check_empty_payload_fake",
    "check_deceptive_double_extension",
    "check_password_trap",
    "check_homoglyph_spoofing",
]
