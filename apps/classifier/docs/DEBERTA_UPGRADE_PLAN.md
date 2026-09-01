# DeBERTa-v3-small Upgrade Plan

**Status:** Evaluation complete — DeBERTa underperforms MiniLM on this task
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
4. **Zero inference changes** — `transformer_onnx_backend.py` already handles optional `token_type_ids` (verified)

---

## Actual Results (Same Training Data, Same Test Set)

Both models trained on `deepseek_labeled/train.jsonl` (5,765 samples), evaluated on `deepseek_labeled/test.jsonl` (638 samples).

| Metric | MiniLM (v2) | DeBERTa (v2) | Winner |
|--------|-------------|--------------|--------|
| Val accuracy (training) | 88.7% | 89.0% | DeBERTa (marginal) |
| **Test accuracy (FP32 ONNX)** | **86.5%** | **78.2%** | **MiniLM** |
| **Test accuracy (INT8 ONNX)** | **84.0%** | **69.6%** | **MiniLM** |
| Quantization loss | 2.5% | 8.6% | MiniLM |
| Macro F1 (INT8) | 0.866 | 0.722 | MiniLM |
| Inference speed | 25 it/s | 12 it/s | MiniLM |
| Model size (INT8) | ~67MB | ~164MB | MiniLM |

**Conclusion:** MiniLM outperforms DeBERTa on this task across all metrics. DeBERTa's `torch.onnx.export` degrades disentangled attention significantly, and INT8 quantization compounds the loss. Recommend keeping MiniLM as production v1.

---

## Training Time Estimate (Mac M1 Pro, 16GB)

| Phase | Time |
|-------|------|
| Model download (440MB, first run) | ~5 min |
| Tokenization | ~2 min |
| Training (4 epochs, ~4,900 samples, max_length=256) | ~26 min |
| ONNX export + INT8 quantization | ~2 min |
| **Total** | **~35 min** |

**CRITICAL:** `max_length` MUST be 256, NOT 512. DeBERTa's O(n²) disentangled attention at 512 causes M1 to freeze. At 256, training completes in ~26 min. At 512, it never finishes (2+ hours, no epoch completion).

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
  max_length: 256                 # CRITICAL: 512 causes O(n^2) disentangled attention to freeze M1
  epochs: 4                       # DeBERTa converges faster than MiniLM; 6 risks overfitting on 5K samples
  learning_rate: 1e-5
  warmup_ratio: 0.15              # 15% warmup to safely ease into disentangled attention layers
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

## Training Data Pipeline

Training data lives in PostgreSQL `labeled_results` table. Export before training:

```bash
# Step 1: Export from PostgreSQL to JSONL (generates train.jsonl + test.jsonl)
python src/export_labeled.py --split --output data/deepseek_labeled/merged.jsonl

# Step 2: Train DeBERTa
python src/train_transformer.py \
  --data data/deepseek_labeled/train.jsonl \
  --out_dir data/models/deberta_v3_small \
  --config config/deberta.yaml
```

Source: `docs/RETRAINING_PLAN.md` — 5,765 labeled samples, 9 categories.

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

## Verified Technical Details

### CRITICAL: max_length=512 freezes M1

DeBERTa's disentangled attention is O(n²). At `max_length=512`, batch_size=16 causes M1 Pro to freeze (never completes epoch 1 in 2+ hours). At `max_length=256`, training completes in ~26 min with batch=16 at ~17 it/s. Benchmark:

| max_length | Speed (batch=16) | Training time (4 epochs) |
|------------|-------------------|--------------------------|
| 128 | 12 it/s | ~27 min |
| 256 | 17 it/s | ~26 min |
| 512 | 3 it/s | **never completes** |

### token_type_ids — No changes needed

`transformer_onnx_backend.py:71-72` already performs a dual check:
```python
if "token_type_ids" in enc and any(i.name == "token_type_ids" for i in self.session.get_inputs()):
    inputs["token_type_ids"] = enc["token_type_ids"].astype(np.int64)
```
DeBERTa-v3 ONNX models do NOT include `token_type_ids` in their input signature. The second condition is `False`, so it is correctly skipped. No crash, no fix required.

### Tokenizer dependencies — sentencepiece only, no protobuf

The `microsoft/deberta-v3-small` repo contains `spm.model` (SentencePiece format), NOT `tokenizer.model`. There is no `tokenizer.json` (fast tokenizer unavailable). `AutoTokenizer` falls back to `DebertaV2Tokenizer` (slow), which imports `sentencepiece` directly. `protobuf` is NOT required — DeBERTa-v3 uses standard SentencePiece binary format, not protobuf format.

### use_fast=False — unnecessary

Since no `tokenizer.json` exists in the repo, `AutoTokenizer` automatically uses the slow tokenizer. Explicitly setting `use_fast=False` is redundant.

### id2label/label2id — nice-to-have

Adding these to `AutoModelForSequenceClassification.from_pretrained()` enables `model.config.id2label` for interpretability. Not required — the `LabelEncoder` joblib handles the actual mapping at inference time.

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

## Risks (Realized)

| Risk | Status | Details |
|------|--------|---------|
| ONNX export degradation | **REALIZED** | `torch.onnx.export` degrades DeBERTa's disentangled attention by 8.3% (86.5% → 78.2%) |
| INT8 quantization | **REALIZED** | Costs DeBERTa 8.6% accuracy (78.2% → 69.6%) vs only 2.5% for MiniLM |
| MPS freeze — max_length=512 | **REALIZED** | Must use max_length=256 or M1 freezes |
| Larger model | REALIZED | 164MB vs 67MB, slower inference (12 it/s vs 25 it/s) |
| No risk to current model | CONFIRMED | MiniLM stays untouched |

---

## Execution Order

1. ~~Add `sentencepiece`, `optimum[exporters]` to `requirements.txt`~~ ✓
2. ~~Create `config/deberta.yaml` (training config)~~ ✓
3. ~~Create `config/deberta_inference.yaml` (inference config)~~ ✓
4. ~~Wire `model_name` from config in `train_transformer.py`~~ ✓
5. ~~Export training data from PostgreSQL → `data/deepseek_labeled/train.jsonl`~~ ✓
6. ~~(Optional) Clean 34 low-confidence labels~~ — skipped
7. ~~Train DeBERTa-v3-small → `data/models/deberta_v3_small/`~~ ✓ (~26 min)
8. ~~Export ONNX INT8 → `data/models/transformer_deberta/`~~ ✓
9. ~~Evaluate both models on test split~~ ✓
10. Commit
