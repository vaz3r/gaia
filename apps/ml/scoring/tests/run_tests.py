"""
Test runner for unit and regression tests.
"""
import sys
from pathlib import Path

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tests.unit.test_silver_rules import (
    test_empty_payload_rule,
    test_deceptive_double_extension,
    test_password_trap,
    test_homoglyph_detection,
    test_sha1_mismatch,
    test_evaluate_silver_invariants_fake,
)
from tests.unit.test_policy_engine import (
    test_stale_swarm_never_marked_as_spam,
    test_malicious_fake_suppression,
    test_password_trap_deduction,
    test_null_name_safety,
)


def run_all_tests():
    tests = [
        ("Silver: Empty Payload Rule", test_empty_payload_rule),
        ("Silver: Deceptive Double Extension", test_deceptive_double_extension),
        ("Silver: Password Trap Detection", test_password_trap),
        ("Silver: Homoglyph Spoofing Detection", test_homoglyph_detection),
        ("Silver: SHA1 Mismatch Flag", test_sha1_mismatch),
        ("Silver: Evaluate Invariants on Fake", test_evaluate_silver_invariants_fake),
        ("Policy: Stale Swarm Never Spam Invariant", test_stale_swarm_never_marked_as_spam),
        ("Policy: Malicious Fake Suppression Invariant", test_malicious_fake_suppression),
        ("Policy: Password Trap Deduction", test_password_trap_deduction),
        ("Policy: Null Name Safety", test_null_name_safety),
    ]

    passed = 0
    print("==================================================")
    print(f"RUNNING {len(tests)} SCORING UNIT & INVARIANT TESTS")
    print("==================================================")

    for name, fn in tests:
        try:
            fn()
            print(f"  [PASS] {name}")
            passed += 1
        except Exception as e:
            print(f"  [FAIL] {name}: {e}")

    print("==================================================")
    print(f"RESULT: {passed}/{len(tests)} tests passed")
    print("==================================================")
    if passed < len(tests):
        sys.exit(1)


if __name__ == "__main__":
    run_all_tests()
