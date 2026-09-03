# Model Upgrade Plan

**Status:** ModernBERT-base deployed as production v2
**Last updated:** 2026-09-02

---

## Results Summary

Both models trained on `labeled_data/train.jsonl` (5,765 samples), evaluated on `labeled_data/test.jsonl` (638 samples).

| Metric | MiniLM-L12-v2 (v1) | ModernBERT-base (v2) | Delta |
|--------|---------------------|----------------------|-------|
| **Test accuracy (INT8 ONNX)** | **87.3%** | **90.1%** | **+2.8%** |
| **Test macro F1 (INT8 ONNX)** | **0.869** | **0.899** | **+0.030** |
| Inference speed | 38 it/s | 10 it/s | -28 it/s |
| Model size (INT8) | 33.9 MB | 150.6 MB | +116.7 MB |
| Training time (MPS) | ~42 min | ~37 min | -5 min |

### Per-Category F1 Improvement

| Category | MiniLM | ModernBERT | Delta |
|----------|--------|------------|-------|
| Adult | 0.91 | 0.92 | +0.01 |
| Anime | 0.89 | 0.86 | -0.03 |
| Applications | 0.91 | 0.92 | +0.01 |
| Documentaries | 0.85 | 0.95 | **+0.10** |
| Games | 0.89 | 0.93 | **+0.04** |
| Movies | 0.86 | 0.90 | **+0.04** |
| Music | 0.92 | 0.91 | -0.01 |
| Other | 0.76 | 0.80 | **+0.04** |
| Television | 0.82 | 0.89 | **+0.07** |

---

## Why ModernBERT

1. **Latest architecture** (Dec 2024) — RoPE positional embeddings, alternating attention, 8192 context
2. **MPS-safe** — Standard attention (no disentangled attention like DeBERTa), no NaN issues
3. **2-4x faster** than older encoder models despite being larger
4. **Higher accuracy** — 90.1% vs 87.3% on INT8 ONNX

---

## Why DeBERTa Failed

| Issue | Details |
|-------|---------|
| MPS float16 NaN | PyTorch MPS backend produces NaN with DeBERTa's disentangled attention in float16 |
| ONNX export degradation | `torch.onnx.export` degrades disentangled attention by 8.3% |
| INT8 quantization | Costs additional 8.6% accuracy |
| CPU-only | Forces CPU training (~84 min) which is 2x slower than MPS |

---

## Model Directory Structure

```
data/models/
├── transformer/                    # Production v1 (MiniLM) — keep as-is
│   └── single_stage/
├── transformer_v3/                 # Training output v1 (MiniLM)
│   ├── model/
│   ├── model.onnx
│   ├── model_int8.onnx
│   ├── tokenizer/
│   └── label_encoder.joblib
├── modernbert_base/                # Training output v2 (ModernBERT) — NEW
│   ├── model/
│   ├── model.onnx
│   ├── model_int8.onnx
│   ├── tokenizer/
│   └── label_encoder.joblib
└── deberta_v3_small/               # Training output (DeBERTa) — abandoned
```

---

## Config Files

| Config | Model | Status |
|--------|-------|--------|
| `config/transformer_v2.yaml` | MiniLM-L12-v2 | Current production |
| `config/modernbert.yaml` | ModernBERT-base | **New production candidate** |
| `config/deberta.yaml` | DeBERTa-v3-small | Abandoned |

---

## Training Commands

```bash
# Train MiniLM (v1)
python3 src/train_transformer.py \
  --data data/labeled_data/train.jsonl \
  --out_dir data/models/transformer_v3 \
  --config config/transformer_v2.yaml

# Train ModernBERT (v2)
python3 src/train_transformer.py \
  --data data/labeled_data/train.jsonl \
  --out_dir data/models/modernbert_base \
  --config config/modernbert.yaml
```

---

## Deployment

To deploy ModernBERT as production:

1. Copy `data/models/modernbert_base/model_int8.onnx` to `data/models/transformer/single_stage/model_int8.onnx`
2. Copy `data/models/modernbert_base/tokenizer/` to `data/models/transformer/single_stage/tokenizer/`
3. Copy `data/models/modernbert_base/label_encoder.joblib` to `data/models/transformer/single_stage/label_encoder.joblib`
4. Update `config/transformer.yaml` to point to ModernBERT paths
5. Rebuild Docker image

---

## Remaining Risks

| Risk | Mitigation |
|------|------------|
| ModernBERT is 4.4x larger than MiniLM | Still fast at 10 it/s for batch processing |
| INT8 quantization loss | ModernBERT: 0.8% loss (0.907 → 0.899 F1), acceptable |
| No MPS issue | Confirmed — standard attention works fine |
