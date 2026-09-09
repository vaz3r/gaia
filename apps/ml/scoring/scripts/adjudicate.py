"""
CLI Utility to Adjudicate and Override Torrent Trust & Risk Decisions.
Allows operators to mark torrents as verified genuine (ALLOW) or confirmed malicious (SUPPRESS),
guaranteeing database integrity, history audit logging, and gold-benchmark inclusion.
"""
import sys
import json
import uuid
import argparse
from pathlib import Path
from datetime import datetime, timezone

# Add parent path for imports
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.data.db import get_db_connection
from src.common.types import PolicyAction, RiskTier, DecisionSource


ACTION_DEFAULTS = {
    PolicyAction.ALLOW.value: {
        "risk_tier": RiskTier.SAFE.value,
        "integrity_score": 100,
        "policy_integrity_score": 100,
    },
    PolicyAction.DOWNRANK.value: {
        "risk_tier": RiskTier.REVIEW.value,
        "integrity_score": 40,
        "policy_integrity_score": 40,
    },
    PolicyAction.SUPPRESS.value: {
        "risk_tier": RiskTier.BLOCKED.value,
        "integrity_score": 0,
        "policy_integrity_score": 0,
    },
    PolicyAction.REVIEW.value: {
        "risk_tier": RiskTier.REVIEW.value,
        "integrity_score": 50,
        "policy_integrity_score": 50,
    },
}


def adjudicate_torrent(
    infohash_hex: str,
    action: str,
    notes: str = "",
    risk_tier: str = None,
    integrity_score: int = None,
    add_to_gold: bool = True,
) -> dict:
    infohash_hex = infohash_hex.strip().lower()
    if len(infohash_hex) != 40:
        raise ValueError(f"Invalid infohash length ({len(infohash_hex)}). Must be a 40-character hex string.")

    action_norm = action.strip().upper()
    if action_norm not in ACTION_DEFAULTS:
        raise ValueError(f"Invalid action '{action}'. Must be one of {list(ACTION_DEFAULTS.keys())}")

    defaults = ACTION_DEFAULTS[action_norm]
    tier_val = (risk_tier or defaults["risk_tier"]).strip().upper()
    score_val = integrity_score if integrity_score is not None else defaults["integrity_score"]

    conn = get_db_connection()
    try:
        with conn.cursor() as cur:
            # 1. Fetch current torrent details
            cur.execute("""
                SELECT encode(infohash, 'hex'), name, total_size, file_count, 
                       integrity_score, risk_tier, policy_action, decision_source
                FROM torrents
                WHERE infohash = decode(%s, 'hex');
            """, (infohash_hex,))
            row = cur.fetchone()
            if not row:
                raise ValueError(f"Torrent with infohash {infohash_hex} not found in database.")

            ih_hex, name, total_size, file_count, old_score, old_tier, old_action, old_source = row

            # 2. Update torrent record with MANUAL decision source
            cur.execute("""
                UPDATE torrents
                SET policy_action = %s,
                    risk_tier = %s,
                    integrity_score = %s,
                    policy_integrity_score = %s,
                    decision_source = 'MANUAL',
                    scored_at = now()
                WHERE infohash = decode(%s, 'hex');
            """, (action_norm, tier_val, score_val, score_val, infohash_hex))

            # 3. Log to torrent_score_history
            reason_codes = [f"MANUAL_ADJUDICATION:{action_norm}"]
            if notes:
                reason_codes.append(f"NOTES:{notes}")

            run_id = str(uuid.uuid4())
            cur.execute("""
                INSERT INTO torrent_score_history (
                    infohash, scoring_run_id, model_name, model_version,
                    model_safe_probability, policy_integrity_score, integrity_score,
                    metadata_quality_score, availability_score, risk_tier,
                    policy_action, decision_source, reason_codes, score_status, scored_at
                ) VALUES (
                    decode(%s, 'hex'), %s, 'manual_adjudication', 'operator_review_v1',
                    %s, %s, %s,
                    100, 100, %s,
                    %s, 'MANUAL', %s::jsonb, 'OVERRIDDEN', now()
                );
            """, (
                infohash_hex, run_id,
                1.0 if action_norm == "ALLOW" else 0.0,
                score_val, score_val,
                tier_val, action_norm,
                json.dumps(reason_codes),
            ))

        conn.commit()
    finally:
        conn.close()

    result = {
        "infohash": infohash_hex,
        "name": name,
        "previous": {
            "action": old_action,
            "risk_tier": old_tier,
            "integrity_score": old_score,
            "decision_source": old_source,
        },
        "updated": {
            "action": action_norm,
            "risk_tier": tier_val,
            "integrity_score": score_val,
            "decision_source": "MANUAL",
            "notes": notes,
        },
    }

    # 4. Optional inclusion in Gold Benchmark for model retraining
    if add_to_gold:
        gold_path = Path(__file__).resolve().parent.parent / "data" / "gold_benchmark.json"
        gold_records = []
        if gold_path.exists():
            try:
                with open(gold_path, "r", encoding="utf-8") as f:
                    gold_records = json.load(f)
            except Exception:
                gold_records = []

        # Check if already in gold
        existing = next((r for r in gold_records if r.get("infohash") == infohash_hex), None)
        record = {
            "infohash": infohash_hex,
            "name": name,
            "total_size": total_size,
            "file_count": file_count,
            "is_safe": 1.0 if action_norm == "ALLOW" else 0.0,
            "action": action_norm,
            "notes": notes,
            "adjudicated_at": datetime.now(timezone.utc).isoformat(),
        }
        if existing:
            existing.update(record)
        else:
            gold_records.append(record)

        try:
            gold_path.parent.mkdir(parents=True, exist_ok=True)
            with open(gold_path, "w", encoding="utf-8") as f:
                json.dump(gold_records, f, indent=2)
            result["gold_benchmark_updated"] = True
        except Exception as e:
            result["gold_benchmark_updated"] = False
            result["gold_benchmark_error"] = str(e)

    return result


def main():
    parser = argparse.ArgumentParser(description="Adjudicate and override torrent quality and risk score.")
    parser.add_argument("--infohash", required=True, help="40-hex infohash of the torrent to override")
    parser.add_argument("--action", required=True, choices=["ALLOW", "DOWNRANK", "SUPPRESS", "REVIEW"],
                        help="New policy action (ALLOW = verified genuine, SUPPRESS = malicious fake)")
    parser.add_argument("--notes", default="", help="Reason or operator notes for the override")
    parser.add_argument("--tier", default=None, choices=["SAFE", "REVIEW", "SUSPICIOUS", "BLOCKED"],
                        help="Optional risk tier override")
    parser.add_argument("--score", type=int, default=None, help="Optional integer integrity score (0-100)")
    parser.add_argument("--no-gold", action="store_true", help="Do not add to gold benchmark training data")

    args = parser.parse_args()

    try:
        res = adjudicate_torrent(
            infohash_hex=args.infohash,
            action=args.action,
            notes=args.notes,
            risk_tier=args.tier,
            integrity_score=args.score,
            add_to_gold=not args.no_gold,
        )
        print(json.dumps(res, indent=2))
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
