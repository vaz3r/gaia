.PHONY: train-mlp train-transformers evaluate-mlp evaluate-transformers export-mlp export-transformers pseudo-label-mlp pseudo-label-transformers export-labeled-mlp export-labeled-transformers test clean

# ── Infrastructure ───────────────────────────────────────────────────────────

test:
	python -m pytest transformers/tests/ mlp/tests/ -v

clean:
	rm -rf transformers/data/models/mlp_* mlp/data/models/*

# ── MLP ──────────────────────────────────────────────────────────────────────

train-mlp:
	cd mlp && $(MAKE) train

evaluate-mlp:
	cd mlp && $(MAKE) evaluate

export-mlp:
	cd mlp && $(MAKE) export

pseudo-label-mlp:
	cd mlp && $(MAKE) pseudo-label

export-labeled-mlp:
	cd mlp && $(MAKE) export-labeled

# ── Transformers ─────────────────────────────────────────────────────────────

train-transformers:
	cd transformers && $(MAKE) train

evaluate-transformers:
	cd transformers && $(MAKE) evaluate

export-transformers:
	cd transformers && $(MAKE) export

pseudo-label-transformers:
	cd transformers && $(MAKE) pseudo-label

export-labeled-transformers:
	cd transformers && $(MAKE) export-labeled

train-transformers-pseudo:
	cd transformers && $(MAKE) train-pseudo
