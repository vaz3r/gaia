#!/usr/bin/env python3
"""
Setup Reviewer A and Reviewer B packages for Gold Pilot v1.
Copies blind records, review templates, and annotation guide while strictly
preserving isolation from manifest, candidate pool, and training data.
"""
import hashlib
import json
import os
import shutil
from pathlib import Path

BASE_DIR = Path("apps/classifier/data/gold_pilot_v1")
GUIDE_SRC = Path("docs/CLASSIFIER_ANNOTATION_GUIDE.md")

BLIND_SRC = BASE_DIR / "gold_pilot_v1_blind.jsonl"
MANIFEST_SRC = BASE_DIR / "gold_pilot_v1_manifest.json"
TEMPLATE_SRC = BASE_DIR / "gold_pilot_v1_review_template.jsonl"

EXPECTED_BLIND = "4b0f65200c23f80ae8c0e41c301c81ef2ec9d85f29de19d8e477f5eb28b0a708"
EXPECTED_MANIFEST = "ea226ab57a9b8e7b95e3d73ab44ee9073cf3ec258176eaa8944099d7482057c5"
EXPECTED_TEMPLATE = "2b74c602db0cce7ad70f0724ca387090f3cf10ec5fe19c7ee45de4e7e4d644cc"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def main():
    # 1. Verify frozen checksums
    assert sha256_file(BLIND_SRC) == EXPECTED_BLIND, f"Blind SHA mismatch: {sha256_file(BLIND_SRC)}"
    assert sha256_file(MANIFEST_SRC) == EXPECTED_MANIFEST, f"Manifest SHA mismatch: {sha256_file(MANIFEST_SRC)}"
    assert sha256_file(TEMPLATE_SRC) == EXPECTED_TEMPLATE, f"Template SHA mismatch: {sha256_file(TEMPLATE_SRC)}"

    rev_a_dir = BASE_DIR / "reviewer_a"
    rev_b_dir = BASE_DIR / "reviewer_b"

    rev_a_dir.mkdir(parents=True, exist_ok=True)
    rev_b_dir.mkdir(parents=True, exist_ok=True)

    # 2. Copy blind files
    shutil.copy2(BLIND_SRC, rev_a_dir / "gold_pilot_v1_blind.jsonl")
    shutil.copy2(BLIND_SRC, rev_b_dir / "gold_pilot_v1_blind.jsonl")

    # 3. Copy review templates to named annotation files
    shutil.copy2(TEMPLATE_SRC, rev_a_dir / "reviewer_a_annotations.jsonl")
    shutil.copy2(TEMPLATE_SRC, rev_b_dir / "reviewer_b_annotations.jsonl")

    # 4. Copy annotation guide
    shutil.copy2(GUIDE_SRC, rev_a_dir / "CLASSIFIER_ANNOTATION_GUIDE.md")
    shutil.copy2(GUIDE_SRC, rev_b_dir / "CLASSIFIER_ANNOTATION_GUIDE.md")

    # 5. Write README.md for Reviewer A
    readme_a_content = """# Gold Pilot v1 — Reviewer A Instructions

Welcome to the Gold Pilot v1 human annotation task.

## Instructions

1. Read `CLASSIFIER_ANNOTATION_GUIDE.md` thoroughly before annotating.
2. Inspect torrent records in `gold_pilot_v1_blind.jsonl`.
3. Record your annotations exclusively in `reviewer_a_annotations.jsonl`.
4. Label every one of the 300 records completely and independently.
5. Do not consult model predictions, previous labels, candidate retrieval groups, the internal manifest, or Reviewer B.
6. Use strictly the frozen eight-class taxonomy:
   - `Anime`, `Applications`, `Documentaries`, `Games`, `Movies`, `Music`, `Other`, `Television`
7. Provide an explanatory `reason` string whenever:
   - `reviewer_confidence` is `low`
   - `ambiguous` is `true`
   - `adjudication_required` is `true`
8. Record your `annotation_timestamp` in ISO 8601 format (e.g. `2026-08-30T10:00:00Z`).
9. Keep `pilot_id` unchanged for every row.
10. Run the validation tool after completing all 300 annotations:

```bash
python3 apps/classifier/tools/validate_gold_annotations.py \\
  apps/classifier/data/gold_pilot_v1/reviewer_a/reviewer_a_annotations.jsonl \\
  --manifest apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json
```
"""
    with open(rev_a_dir / "README.md", "w", encoding="utf-8") as f:
        f.write(readme_a_content)

    # 6. Write README.md for Reviewer B
    readme_b_content = """# Gold Pilot v1 — Reviewer B Instructions

Welcome to the Gold Pilot v1 human annotation task.

## Instructions

1. Read `CLASSIFIER_ANNOTATION_GUIDE.md` thoroughly before annotating.
2. Inspect torrent records in `gold_pilot_v1_blind.jsonl`.
3. Record your annotations exclusively in `reviewer_b_annotations.jsonl`.
4. Label every one of the 300 records completely and independently.
5. Do not consult model predictions, previous labels, candidate retrieval groups, the internal manifest, or Reviewer A.
6. Use strictly the frozen eight-class taxonomy:
   - `Anime`, `Applications`, `Documentaries`, `Games`, `Movies`, `Music`, `Other`, `Television`
7. Provide an explanatory `reason` string whenever:
   - `reviewer_confidence` is `low`
   - `ambiguous` is `true`
   - `adjudication_required` is `true`
8. Record your `annotation_timestamp` in ISO 8601 format (e.g. `2026-08-30T10:00:00Z`).
9. Keep `pilot_id` unchanged for every row.
10. Run the validation tool after completing all 300 annotations:

```bash
python3 apps/classifier/tools/validate_gold_annotations.py \\
  apps/classifier/data/gold_pilot_v1/reviewer_b/reviewer_b_annotations.jsonl \\
  --manifest apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json
```
"""
    with open(rev_b_dir / "README.md", "w", encoding="utf-8") as f:
        f.write(readme_b_content)

    print("SUCCESS: Reviewer packages created.")


if __name__ == "__main__":
    main()
