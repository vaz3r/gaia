# Classifier Retraining Plan

**Status:** Waiting for labeling to complete
**Last updated:** 2026-09-01

---

## Current State

| Metric | Value |
|--------|-------|
| Total labeled | 5,422 |
| DeepSeek (Expert) | 2,334 |
| Gemini (MCP) | 3,088 |
| Target | ~6,000 true labels |
| Remaining | ~578 |

### Distribution

| Category | Count | Status |
|----------|-------|--------|
| Adult | 1,223 | Over-indexed |
| Television | 609 | Good |
| Anime | 539 | Good |
| Music | 527 | Good |
| Games | 512 | Good |
| Documentaries | 509 | Good |
| Other | 505 | Good |
| Movies | 502 | Good |
| Applications | 496 | Good |

---

## Pipeline

### Phase 1: Labeling (IN PROGRESS)

- Store labels in PostgreSQL `labeled_results` table
- Use `deepseek/classify.py` with Expert model (batch size 50)
- Balanced targeting picks underrepresented categories
- **Action:** Run more DeepSeek batches until ~6,000 total

```bash
cd apps/classifier/deepseek
source venv/bin/activate
python classify.py --loops 12    # ~600 more labels
```

### Phase 2: Export

- `src/export_labeled.py` — exports from PostgreSQL to JSONL
- Joins `labeled_results` with `torrents` for full metadata
- Splits into train (90%) / test (10%)

```bash
python src/export_labeled.py --split --output data/deepseek_labeled/merged.jsonl
```

### Phase 3: Retrain

- `src/train_transformer.py` — trains `all-MiniLM-L12-v2` on exported data
- Uses class-weighted loss, stratified split, best-model checkpointing
- Config: `config/transformer_v2.yaml` (raw mode, metadata only)

```bash
python src/train_transformer.py --data data/deepseek_labeled/train.jsonl --config config/transformer_v2.yaml
```

### Phase 4: Evaluate

- Run on `data/manual_eval_set_balanced_2000.jsonl`
- Target: Macro F1 >= 0.78 (current: 0.712)
- Also run on natural test set `data/manual_eval_set_1000.jsonl`

### Phase 5: Export ONNX

- `src/export_onnx.py` — quantize to INT8 ONNX for production
- Target: `data/models/transformer/single_stage/model_int8.onnx`

---

## Rollback

If bad labels are inserted:

```sql
-- Remove all DeepSeek labels
DELETE FROM labeled_results WHERE source = 'deepseek';

-- Remove last N labels
DELETE FROM labeled_results
WHERE source = 'deepseek'
AND labeled_at > (
    SELECT labeled_at FROM labeled_results
    WHERE source='deepseek'
    ORDER BY labeled_at DESC LIMIT 1 OFFSET <N-1>
);
```

---

## Notes

- DeepSeek Expert batch size: 50 (reliable max, no duplicates)
- MCP batch size: 100 (for Gemini Spark, but content policy blocks adult content)
- Gemini Spark is unreliable — prefer DeepSeek for labeling
- Both write to same table, safe to run sequentially (not parallel)
