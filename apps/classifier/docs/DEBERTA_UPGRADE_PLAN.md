# DeBERTa-v3-small Upgrade Plan

**Status:** Pending approval
**Last updated:** 2026-09-01

---

## Goal

Upgrade from `sentence-transformers/all-MiniLM-L12-v2` (33M params) to `microsoft/deberta-v3-small` (44M params) to push accuracy from 90.1% to 93-96%.

---

## Why DeBERTa-v3-small

1. **Tokenization** — SentencePiece (BPE) handles torrent names (`x264`, `1080p`, `[SubsPlease]`) much better than MiniLM's WordPiece, which aggressively fractures these into meaningless subwords
2. **Disentangled attention** — Key architectural innovation excels at understanding token relationships in dense, technical text
3. **Sweet spot** — ~44M params (only 33% larger than MiniLM), but vastly superior representations
4. **Zero inference changes** — `transformer_onnx_backend.py` already handles optional `token_type_ids`

---

## Expected Outcome

| Metric | Current (MiniLM) | Expected (DeBERTa) |
|--------|-------------------|---------------------|
| Accuracy | 90.1% | 93-96% |
| Macro F1 | 0.896 | 0.93-0.96 |
| Inference speed | ~200 it/s | ~80-100 it/s |
| Model size (INT8) | ~33MB | ~90-110MB |

---

## Files to Change

| File | Change | Difficulty |
|------|--------|------------|
| `config/transformer_v2.yaml` | Update `model_name`, increase `max_length` to 512, lower LR to 1e-5 | Trivial |
| `src/train_transformer.py` | Wire config `model_name` into tokenizer/model loading (lines 144, 162-163 are hardcoded) | Easy |
| `src/classify_batch.py` | Update model metadata string `minilm-l12-int8` → `deberta-v3-small-int8` | Trivial |
| `requirements.txt` | Add `optimum[exporters]`, `sentencepiece`, `torch` | Easy |
| `docs/CLASSIFIER.md` | Update model name references | Trivial |
| `docs/RETRAINING_PLAN.md` | Update model name reference | Trivial |

### Files that do NOT change

- `src/core/text_builder.py` — model-agnostic
- `src/backends/transformer_onnx_backend.py` — already handles optional `token_type_ids`
- `src/export_onnx.py` — model-agnostic, optimum-cli supports DeBERTa-v3 natively
- `Dockerfile` — paths are generic

---

## Config Changes

```yaml
# config/transformer_v2.yaml
transformer:
  model_name: microsoft/deberta-v3-small
  batch_size: 16
  max_length: 512       # was 256 — DeBERTa handles longer sequences better
  epochs: 6             # was 4 — give DeBERTa more time to converge
  learning_rate: 1e-5   # was 2e-5 — DeBERTa needs gentler fine-tuning
  weight_decay: 0.01
  grad_clip: 1.0
```

---

## Optional: Clean Low-Confidence Labels

34 labels with `confidence=low` are likely noise. Remove before retraining:

| Category | Low Confidence Count |
|----------|---------------------|
| Other | 30 |
| Adult | 2 |
| Movies | 2 |

```sql
DELETE FROM labeled_results WHERE confidence = 'low';
```

---

## Risks

| Risk | Mitigation |
|------|------------|
| Tokenizer mismatch — loading old MiniLM tokenizer with DeBERTa model | Training script saves tokenizer from same checkpoint; verified safe |
| Larger model — ~142M params (vs 33M MiniLM), ~3-4x bigger ONNX | Still fast at 80-100 it/s, acceptable for batch processing |
| Higher memory — 512 max_length vs 256 | Batch size 16 is conservative, should be fine on MPS/CPU |
| Learning rate — DeBERTa needs lower LR | Using 1e-5 (current 2e-5 may be too high) |

---

## Execution Order

1. Wire `model_name` from config into `train_transformer.py`
2. Update config with DeBERTa settings
3. (Optional) Clean 34 low-confidence labels
4. Train DeBERTa-v3-small
5. Evaluate on test split
6. Export INT8 ONNX
7. Update production model
8. Commit
