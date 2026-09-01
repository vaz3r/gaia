# DeBERTa-v3-small Upgrade Plan

**Status:** Pending approval
**Last updated:** 2026-09-01
**Approach:** Dual model — keep current MiniLM, branch out with DeBERTa

---

## Goal

Keep the current MiniLM model (90.1% accuracy) as production v1. Train DeBERTa-v3-small as a separate model in its own directory, evaluate it, and deploy as production v2 if it outperforms MiniLM.

---

## Why DeBERTa-v3-small

1. **Tokenization** — SentencePiece (BPE) handles torrent names (`x264`, `1080p`, `[SubsPlease]`) much better than MiniLM's WordPiece, which aggressively fractures these into meaningless subwords
2. **Disentangled attention** — Key architectural innovation excels at understanding token relationships in dense, technical text
3. **Sweet spot** — ~44M params (only 33% larger than MiniLM), but vastly superior representations
4. **Zero inference changes** — `transformer_onnx_backend.py` already handles optional `token_type_ids`

---

## Expected Outcome

| Metric | MiniLM (v1) | DeBERTa (v2) |
|--------|-------------|--------------|
| Accuracy | 90.1% | 93-96% |
| Macro F1 | 0.896 | 0.93-0.96 |
| Inference speed | ~200 it/s | ~80-100 it/s |
| Model size (INT8) | ~33MB | ~90-110MB |

---

## Dual Model Directory Structure

```
data/models/
├── transformer/                    # Production v1 (MiniLM) — keep as-is
│   └── single_stage/
│       ├── model_int8.onnx
│       ├── tokenizer/
│       └── label_encoder.joblib
├── transformer_v2/                 # Training output v1 (MiniLM) — keep as-is
│   ├── model/
│   ├── model.onnx
│   ├── model_int8.onnx
│   ├── tokenizer/
│   └── label_encoder.joblib
├── deberta_v3_small/               # Training output v2 (DeBERTa) — NEW
│   ├── model/
│   ├── model.onnx
│   ├── model_int8.onnx
│   ├── tokenizer/
│   └── label_encoder.joblib
└── transformer_deberta/            # Production v2 (DeBERTa) — NEW
    ├── model_int8.onnx
    ├── tokenizer/
    └── label_encoder.joblib
```

---

## Config Files

```
config/
├── transformer.yaml                # Inference v1 (MiniLM) — keep as-is
├── transformer_v2.yaml             # Training v1 (MiniLM) — keep as-is
├── deberta.yaml                    # Training v2 (DeBERTa) — NEW
└── deberta_inference.yaml          # Inference v2 (DeBERTa) — NEW
```

### `config/deberta.yaml` (training)

```yaml
text_builder:
  max_name_chars: 300

transformer:
  model_name: microsoft/deberta-v3-small
  batch_size: 16
  max_length: 512
  epochs: 6
  learning_rate: 1e-5
  warmup_ratio: 0.1
  weight_decay: 0.01
  grad_clip: 1.0

classifier:
  confidence_threshold: 0.45
```

### `config/deberta_inference.yaml` (inference)

```yaml
transformer:
  model_path: data/models/transformer_deberta/model_int8.onnx
  tokenizer_path: data/models/transformer_deberta/tokenizer
  encoder_path: data/models/transformer_deberta/label_encoder.joblib
  max_length: 512
  batch_size: 16

text_builder:
  max_name_chars: 300
```

---

## Files to Change

| File | Change | Difficulty |
|------|--------|------------|
| `config/deberta.yaml` | New training config for DeBERTa-v3-small | Trivial |
| `config/deberta_inference.yaml` | New inference config for DeBERTa | Trivial |
| `src/train_transformer.py` | Wire `model_name` from config (lines 144, 162-163 are hardcoded) | Easy |
| `requirements.txt` | Add `optimum[exporters]`, `sentencepiece` | Easy |

### Files that do NOT change

- `data/models/transformer/` — production MiniLM stays untouched
- `config/transformer.yaml` — inference config stays as-is
- `config/transformer_v2.yaml` — training config stays as-is
- `src/backends/transformer_onnx_backend.py` — already handles optional `token_type_ids`
- `src/core/text_builder.py` — model-agnostic
- `src/export_onnx.py` — model-agnostic, optimum-cli supports DeBERTa-v3 natively
- `Dockerfile` — paths are generic, works with either model

---

## Training Flow

```bash
# Train MiniLM (existing — don't touch)
python src/train_transformer.py \
  --data data/deepseek_labeled/train.jsonl \
  --out_dir data/models/transformer_v2 \
  --config config/transformer_v2.yaml

# Train DeBERTa (new — separate directory)
python src/train_transformer.py \
  --data data/deepseek_labeled/train.jsonl \
  --out_dir data/models/deberta_v3_small \
  --config config/deberta.yaml
```

## Inference Flow

```bash
# Run with MiniLM (existing)
python src/classify_batch.py \
  --input data/test.jsonl \
  --output data/preds_minilm.jsonl \
  --config config/transformer.yaml \
  --mode transformer

# Run with DeBERTa (new)
python src/classify_batch.py \
  --input data/test.jsonl \
  --output data/preds_deberta.jsonl \
  --config config/deberta_inference.yaml \
  --mode transformer
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
| No risk to current model | Current MiniLM is never modified; lives in separate directories |

---

## Execution Order

1. Create `config/deberta.yaml` (training config)
2. Create `config/deberta_inference.yaml` (inference config)
3. Wire `model_name` from config in `train_transformer.py`
4. (Optional) Clean 34 low-confidence labels
5. Train DeBERTa-v3-small → `data/models/deberta_v3_small/`
6. Evaluate both models on test split
7. Export INT8 ONNX → `data/models/transformer_deberta/`
8. Commit
