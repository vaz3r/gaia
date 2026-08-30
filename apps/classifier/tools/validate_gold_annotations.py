#!/usr/bin/env python3
"""
Gold Annotation Validator.
Validates human annotation files against the frozen gold pilot manifest,
enforcing taxonomy integrity, confidence levels, reasoning requirements, and field privacy.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter
from datetime import datetime
from pathlib import Path

FROZEN_TAXONOMY = {
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
}

ALLOWED_CONFIDENCE = {"high", "medium", "low"}

PROHIBITED_FIELDS = {
    "source_dataset",
    "source_heuristic_label",
    "source_label",
    "model_prediction",
    "model_confidence",
    "predicted_category",
    "prediction",
    "is_pseudo",
    "infohash",
    "candidate_retrieval_category",
}


def validate_gold_annotations(annotation_path: str, manifest_path: str) -> bool:
    """Validate an annotation file against the manifest."""
    print("=================================================================")
    print("VALIDATING GOLD PILOT ANNOTATIONS")
    print("=================================================================")
    print(f"Annotation File: {annotation_path}")
    print(f"Manifest File:   {manifest_path}")

    if not os.path.exists(annotation_path):
        print(f"❌ Error: Annotation file not found: {annotation_path}")
        return False
    if not os.path.exists(manifest_path):
        print(f"❌ Error: Manifest file not found: {manifest_path}")
        return False

    with open(manifest_path, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    manifest_pilot_ids = {r["pilot_id"] for r in manifest.get("records", [])}
    expected_total = len(manifest_pilot_ids)
    print(f"Loaded {expected_total} expected pilot IDs from manifest.\n")

    errors = []
    seen_pilot_ids = set()
    rows = []

    class_counts = Counter()
    confidence_counts = Counter()
    ambiguous_count = 0
    adjudication_count = 0
    reviewers = set()

    with open(annotation_path, "r", encoding="utf-8") as f:
        for line_num, line in enumerate(f, 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except Exception as e:
                errors.append(f"Line {line_num}: Malformed JSON - {e}")
                continue

            rows.append(row)

            # 1. Prohibited field check
            found_prohibited = [k for k in row.keys() if k in PROHIBITED_FIELDS]
            if found_prohibited:
                errors.append(f"Line {line_num}: Prohibited leak fields present: {found_prohibited}")

            # 2. Pilot ID validation
            pid = row.get("pilot_id")
            if not pid:
                errors.append(f"Line {line_num}: Missing pilot_id")
                continue
            if pid in seen_pilot_ids:
                errors.append(f"Line {line_num}: Duplicate pilot_id: {pid}")
            seen_pilot_ids.add(pid)

            if pid not in manifest_pilot_ids:
                errors.append(f"Line {line_num}: Unknown pilot_id not in manifest: {pid}")

            # 3. Label Category validation
            cat = row.get("label_category")
            if cat not in FROZEN_TAXONOMY:
                errors.append(f"Line {line_num} (ID: {pid}): Invalid label_category '{cat}'. Must be one of {sorted(list(FROZEN_TAXONOMY))}")
            else:
                class_counts[cat] += 1

            # 4. Reviewer Confidence validation
            conf = row.get("reviewer_confidence")
            if conf not in ALLOWED_CONFIDENCE:
                errors.append(f"Line {line_num} (ID: {pid}): Invalid reviewer_confidence '{conf}'. Must be 'high', 'medium', or 'low'")
            else:
                confidence_counts[conf] += 1

            # 5. Ambiguous flag validation
            amb = row.get("ambiguous")
            if not isinstance(amb, bool):
                errors.append(f"Line {line_num} (ID: {pid}): 'ambiguous' must be a boolean (true/false)")
            elif amb:
                ambiguous_count += 1

            # 6. Alternate category validation
            alt = row.get("alternate_category")
            if alt is not None:
                if alt not in FROZEN_TAXONOMY:
                    errors.append(f"Line {line_num} (ID: {pid}): Invalid alternate_category '{alt}'.")
                if alt == cat:
                    errors.append(f"Line {line_num} (ID: {pid}): alternate_category '{alt}' cannot be identical to primary category '{cat}'.")

            # 7. Reason requirement validation
            reason = row.get("reason")
            if (conf == "low" or amb is True) and (not reason or not str(reason).strip()):
                errors.append(f"Line {line_num} (ID: {pid}): 'reason' string is required when confidence is 'low' or ambiguous is true.")

            # 8. Reviewer ID validation
            rid = row.get("reviewer_id")
            if not rid or not str(rid).strip():
                errors.append(f"Line {line_num} (ID: {pid}): Missing or empty reviewer_id")
            else:
                reviewers.add(str(rid).strip())

            # 9. Timestamp validation
            ts = row.get("annotation_timestamp")
            if not ts:
                errors.append(f"Line {line_num} (ID: {pid}): Missing annotation_timestamp")
            else:
                try:
                    datetime.fromisoformat(str(ts).replace("Z", "+00:00"))
                except Exception:
                    errors.append(f"Line {line_num} (ID: {pid}): Invalid ISO 8601 timestamp '{ts}'")

            # 10. Adjudication required validation
            adj = row.get("adjudication_required")
            if not isinstance(adj, bool):
                errors.append(f"Line {line_num} (ID: {pid}): 'adjudication_required' must be a boolean (true/false)")
            elif adj:
                adjudication_count += 1

    # Check for missing pilot records
    missing_pids = manifest_pilot_ids - seen_pilot_ids
    if missing_pids:
        errors.append(f"Missing {len(missing_pids)} pilot records from manifest (e.g. {sorted(list(missing_pids))[:3]}...)")

    # Output validation report
    print("-----------------------------------------------------------------")
    print(f"Total Rows Processed:   {len(rows)} / {expected_total}")
    print(f"Reviewers:              {sorted(list(reviewers))}")
    print(f"Class Distribution:     {dict(class_counts)}")
    print(f"Confidence Breakdown:   {dict(confidence_counts)}")
    print(f"Ambiguous Rows:         {ambiguous_count} ({ambiguous_count / max(len(rows), 1) * 100:.1f}%)")
    print(f"Adjudication Required:  {adjudication_count}")
    print("-----------------------------------------------------------------")

    if errors:
        print(f"\n❌ VALIDATION FAILED with {len(errors)} error(s):")
        for err in errors[:25]:
            print(f"   - {err}")
        if len(errors) > 25:
            print(f"   ... and {len(errors) - 25} more errors.")
        return False

    print("\n✅ VALIDATION PASSED: All records strictly conform to taxonomy and manifest rules.")
    return True


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Validate gold pilot annotations against manifest.")
    parser.add_argument("annotation_file", help="Path to annotated JSONL file")
    parser.add_argument("--manifest", default="apps/classifier/data/gold_pilot/gold_pilot_manifest.json", help="Path to gold pilot manifest JSON")
    args = parser.parse_args()
    success = validate_gold_annotations(args.annotation_file, args.manifest)
    sys.exit(0 if success else 1)
