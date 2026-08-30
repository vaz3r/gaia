# Gold Pilot v1 — Reviewer B Instructions

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
python3 apps/classifier/tools/validate_gold_annotations.py \
  apps/classifier/data/gold_pilot_v1/reviewer_b/reviewer_b_annotations.jsonl \
  --manifest apps/classifier/data/gold_pilot_v1/gold_pilot_v1_manifest.json
```
