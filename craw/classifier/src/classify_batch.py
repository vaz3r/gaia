#!/usr/bin/env python3
"""Batch classification CLI for torrent metadata."""

from __future__ import annotations

import argparse
import json
import logging
import sys
import time
from pathlib import Path

import yaml

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
logger = logging.getLogger(__name__)


def load_config(config_path: str) -> dict:
    with open(config_path, encoding="utf-8") as f:
        return yaml.safe_load(f)


def load_torrents(path: str, limit: int | None = None) -> list[dict]:
    torrents = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                torrents.append(json.loads(line))
            if limit and len(torrents) >= limit:
                break
    return torrents


def run_embedding_mode(torrents: list[dict], config: dict, output_f):
    from src.backends.embedding_backend import EmbeddingBackend
    from src.core.text_builder import build_input_text
    import joblib
    import numpy as np

    emb_cfg = config.get("embedding", {})
    cls_cfg = config.get("classifier", {})

    backend = EmbeddingBackend(emb_cfg.get("model_name", "BAAI/bge-small-en-v1.5"), cache_dir=emb_cfg.get("cache_dir"))

    clf = None
    le = None
    model_path = cls_cfg.get("model_path", "data/models/logreg_category.joblib")
    encoder_path = cls_cfg.get("encoder_path", "data/models/label_encoder.joblib")
    if Path(model_path).exists():
        clf = joblib.load(model_path)
        le = joblib.load(encoder_path)
        logger.info("Loaded linear classifier from %s", model_path)
    else:
        logger.warning("No trained classifier at %s, using anchor fallback only", model_path)

    conf_threshold = cls_cfg.get("confidence_threshold", 0.6)
    anchor_threshold = cls_cfg.get("anchor_threshold", 0.5)

    model_name = emb_cfg.get("model_name", "BAAI/bge-small-en-v1.5")
    classifier_name = Path(model_path).stem if clf else "none"

    texts = [build_input_text(row, config) for row in torrents]
    logger.info("Embedding %d texts...", len(texts))
    t0 = time.time()
    all_embeddings = backend.embed(texts, batch_size=emb_cfg.get("batch_size", 64))
    logger.info("Embedded in %.1fs (%.1fms/torrent)", time.time() - t0, (time.time() - t0) / len(texts) * 1000)

    anchor_embeddings = None
    anchor_categories = None
    anchor_file = cls_cfg.get("anchor_file", "data/anchors.json")
    if Path(anchor_file).exists():
        import json
        with open(anchor_file, encoding="utf-8") as f:
            anchor_data = json.load(f)
        anchor_texts = []
        anchor_categories_list = []
        for entry in anchor_data:
            for anchor in entry["anchors"]:
                anchor_texts.append(anchor)
                anchor_categories_list.append(entry["category"])
        anchor_embeddings = backend.embed(anchor_texts)
        anchor_categories = np.array(anchor_categories_list)
        logger.info("Embedded %d anchors", len(anchor_texts))

    n_total = len(torrents)
    n_linear = 0
    n_anchor = 0
    n_fallback = 0
    t0 = time.time()

    for i, row in enumerate(torrents):
        vec = all_embeddings[i]
        category = "Other"
        confidence = 0.0
        method = "fallback"
        top_candidates = []

        if clf is not None:
            probs = clf.predict_proba(vec.reshape(1, -1))[0]
            classes = le.classes_
            top_idx = np.argsort(probs)[::-1]
            top_candidates = [
                {"category": classes[idx], "confidence": round(float(probs[idx]), 4)}
                for idx in top_idx[:3]
            ]
            best_idx = top_idx[0]
            best_prob = float(probs[best_idx])
            if best_prob >= conf_threshold:
                category = classes[best_idx]
                confidence = round(best_prob, 4)
                method = "linear"
                n_linear += 1

        if method == "fallback" and anchor_embeddings is not None:
            sims = np.dot(anchor_embeddings, vec)
            best_idx = int(np.argmax(sims))
            best_sim = float(sims[best_idx])
            if best_sim >= anchor_threshold:
                category = anchor_categories[best_idx]
                confidence = round(best_sim, 4)
                method = "anchor"
                n_anchor += 1
            else:
                n_fallback += 1
        elif method == "fallback":
            n_fallback += 1

        entry = {
            "infohash": row.get("infohash", row.get("id", "")),
            "category": category,
            "confidence": confidence,
            "method": method,
            "model": model_name,
            "classifier": classifier_name,
        }
        if top_candidates:
            entry["top_candidates"] = top_candidates

        output_f.write(json.dumps(entry) + "\n")
        output_f.flush()

    elapsed = time.time() - t0
    logger.info("[%d/%d] linear=%d anchor=%d fallback=%d time=%.1fs",
                n_total, n_total, n_linear, n_anchor, n_fallback, elapsed)

    print(f"\n=== Classification Complete ===")
    print(f"Total:        {n_total}")
    print(f"Linear:       {n_linear} ({n_linear/n_total*100:.1f}%)")
    print(f"Anchor:       {n_anchor} ({n_anchor/n_total*100:.1f}%)")
    print(f"Fallback:     {n_fallback} ({n_fallback/n_total*100:.1f}%)")
    print(f"Total time:   {elapsed:.1f}s")
    print(f"Throughput:   {n_total/elapsed:.0f} it/s")


def run_llm_mode(torrents: list[dict], config: dict, output_f, retry: bool = False):
    from src.core.prompt_builder import PromptBuilder
    from src.core.types import TorrentInput
    from src.core.xml_guardrails import parse_classification_xml
    from src.backends.transformers_backend import TransformersBackend

    backend = TransformersBackend(config)
    prompt_builder = PromptBuilder(config)

    n_total = len(torrents)
    n_success = 0
    n_parse_fail = 0
    total_latency = 0.0
    t0 = time.time()

    for i, row in enumerate(torrents):
        torrent = TorrentInput(
            infohash=row.get("infohash", row.get("id", "")),
            name=row.get("name", ""),
            file_count=row.get("file_count", 0),
            total_size_bytes=row.get("total_size", row.get("total_size_bytes", 0)),
            files=row.get("top_dirs", row.get("files", [])) or [],
        )
        t_start = time.time()

        messages = prompt_builder.build_messages(torrent)
        raw_output = backend.generate(messages)
        result = parse_classification_xml(raw_output)
        parse_status = "success"
        retry_used = False

        if result is None and retry:
            retry_messages = prompt_builder.build_retry_messages(torrent)
            raw_output = backend.generate(retry_messages)
            result = parse_classification_xml(raw_output)
            retry_used = True

        if result is None:
            parse_status = "parse_fail"
            n_parse_fail += 1
            category = "Other"
            confidence = 0.0
        else:
            category = result.category
            confidence = result.confidence
            n_success += 1

        latency_ms = (time.time() - t_start) * 1000
        total_latency += latency_ms

        entry = {
            "infohash": torrent.infohash,
            "category": category,
            "confidence": confidence,
            "method": "llm",
            "model": config.get("model", {}).get("name_or_path", "unknown"),
            "classifier": "llm",
            "raw_output": raw_output,
            "parse_status": parse_status,
            "retry_used": retry_used,
            "latency_ms": round(latency_ms, 1),
        }
        output_f.write(json.dumps(entry) + "\n")
        output_f.flush()

        if (i + 1) % 10 == 0 or i == n_total - 1:
            elapsed = time.time() - t0
            rate = (i + 1) / elapsed if elapsed > 0 else 0
            avg_lat = total_latency / (i + 1)
            logger.info(
                "[%d/%d] %.1f it/s | success=%d fail=%d | avg_latency=%.0fms",
                i + 1, n_total, rate, n_success, n_parse_fail, avg_lat,
            )

    elapsed = time.time() - t0
    avg_latency = total_latency / n_total if n_total > 0 else 0

    print(f"\n=== Classification Complete (LLM) ===")
    print(f"Total:        {n_total}")
    print(f"Success:      {n_success} ({n_success/n_total*100:.1f}%)")
    print(f"Parse fail:   {n_parse_fail} ({n_parse_fail/n_total*100:.1f}%)")
    print(f"Avg latency:  {avg_latency:.0f}ms")
    print(f"Total time:   {elapsed:.1f}s")
    print(f"Throughput:   {n_total/elapsed:.1f} it/s")


def main():
    parser = argparse.ArgumentParser(description="Torrent metadata batch classifier")
    parser.add_argument("--input", required=True, help="Input JSONL file")
    parser.add_argument("--output", required=True, help="Output JSONL file")
    parser.add_argument("--config", default="config/embedding.yaml", help="Config YAML")
    parser.add_argument("--mode", choices=["embedding", "llm"], default="embedding",
                        help="Classification mode")
    parser.add_argument("--limit", type=int, default=None, help="Max torrents to classify")
    parser.add_argument("--retry", action="store_true", help="Retry failed parses (LLM mode)")
    args = parser.parse_args()

    config = load_config(args.config)
    torrents = load_torrents(args.input, args.limit)
    logger.info("Loaded %d torrents from %s", len(torrents), args.input)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as out_f:
        if args.mode == "embedding":
            run_embedding_mode(torrents, config, out_f)
        else:
            run_llm_mode(torrents, config, out_f, retry=args.retry)

    print(f"Output: {args.output}")


if __name__ == "__main__":
    main()
