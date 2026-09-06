#!/usr/bin/env python3
"""
Active Learning Round 1: Label 3K uncertain + audiobook samples via DeepSeek.
"""
import argparse
import json
import logging
import os
import random
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from deepseek import DeepSeekClient
from deepseek.pow_obscura import ObscuraSolver
from classify import CATEGORY_LABELS, CATEGORY_PATTERNS

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

CLASSIFICATION_PROMPT = """You are a torrent file classifier. Classify each torrent into ONE of these 11 categories:

{categories}

## CRITICAL RULES
- You MUST use ONLY these exact category names: Adult, Anime, Applications, Audiobooks, Books & Learning, Documentaries, Games, Movies, Music, Television, Other
- Do NOT invent new category names like "software", "education", etc.
- Base classification on name + file list
- "Other" is a last resort — use only if nothing else fits

## Response Format (one line per torrent, NO extra text)
INFOHASH: <40-char-hex> | LABEL: <category> | CONFIDENCE: <high/medium/low> | REASON: <brief>

## Torrents to classify:
{batch_text}"""


def build_prompt(batch):
    categories_text = "\n".join(f"- **{cat}**: {', '.join(CATEGORY_PATTERNS.get(cat, [])[:5])}" 
                                 for cat in CATEGORY_LABELS)
    
    batch_lines = []
    for item in batch:
        ih = item.get("infohash", "")
        name = item.get("name", "Unknown")[:200]
        file_count = item.get("file_count", 0)
        total_size = item.get("total_size", 0)
        
        files_raw = item.get("files_raw", [])
        if not files_raw:
            files_raw = item.get("files", [])
        
        extensions = []
        file_names = []
        if isinstance(files_raw, list):
            for f in files_raw[:50]:
                if isinstance(f, dict):
                    fname = f.get("name", f.get("path", ""))
                    fsize = f.get("size", 0)
                    if fname:
                        if isinstance(fname, list):
                            fname = "/".join(str(p) for p in fname)
                        file_names.append(str(fname)[:100])
                        ext = str(fname).rsplit(".", 1)[-1].lower() if "." in str(fname) else ""
                        if ext and len(ext) <= 10:
                            extensions.append(ext)
        
        ext_list = ", ".join(sorted(set(extensions))[:10])
        
        batch_lines.append(f"[{ih}] {name} | {file_count} files, {total_size} bytes | Exts: {ext_list}")
        if file_names:
            batch_lines.append(f"  Files: {', '.join(file_names[:15])}")
    
    batch_text = "\n".join(batch_lines)
    
    return CLASSIFICATION_PROMPT.format(
        categories=categories_text,
        batch_text=batch_text
    )


def parse_response(text):
    results = []
    for line in text.strip().split("\n"):
        line = line.strip()
        if not line:
            continue
        
        # Try both formats: with and without INFOHASH: prefix
        if "INFOHASH:" in line:
            parts = line.split("INFOHASH:")[1]
            ih = parts.split("|")[0].strip()
        else:
            # Format: <40-char-hex> | LABEL: ...
            ih = line.split("|")[0].strip() if "|" in line else ""
            if len(ih) != 40 or not all(c in "0123456789abcdef" for c in ih.lower()):
                continue
        
        try:
            label_part = line.split("LABEL:")[1] if "LABEL:" in line else ""
            label = label_part.split("|")[0].strip()
            
            conf_part = line.split("CONFIDENCE:")[1] if "CONFIDENCE:" in line else ""
            confidence = conf_part.split("|")[0].strip().lower()
            
            reason_part = line.split("REASON:")[1] if "REASON:" in line else ""
            reason = reason_part.strip()
            
            if len(ih) == 40 and label in CATEGORY_LABELS:
                results.append({
                    "infohash": ih.lower(),
                    "label_category": label,
                    "confidence": confidence,
                    "reason": reason,
                })
        except Exception:
            continue
    
    return results


def save_to_database(results):
    import psycopg2
    import psycopg2.extras
    
    DB_CONFIG = {
        "host": "workspace-production", "port": 5432, "dbname": "craw",
        "user": "crawler",
        "password": "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b",
        "connect_timeout": 10,
    }
    
    conn = psycopg2.connect(**DB_CONFIG)
    try:
        with conn.cursor() as cur:
            values = []
            for r in results:
                ih = r.get("infohash", "").strip()
                cat = r.get("label_category", "")
                conf = r.get("confidence", "low")
                reason = r.get("reason", "")
                
                if not ih or len(ih) != 40:
                    continue
                if cat not in CATEGORY_LABELS:
                    continue
                
                try:
                    bytea_ih = bytes.fromhex(ih)
                except ValueError:
                    continue
                
                values.append((bytea_ih, cat, conf, reason))
            
            if not values:
                return 0
            
            psycopg2.extras.execute_values(
                cur,
                """
                INSERT INTO labeled_results (infohash, label_category, confidence, reason, labeled_at, source)
                VALUES %s
                ON CONFLICT (infohash) DO UPDATE SET
                    label_category = EXCLUDED.label_category,
                    confidence = EXCLUDED.confidence,
                    reason = EXCLUDED.reason,
                    labeled_at = now(),
                    source = 'deepseek'
                """,
                values,
                template="(%s, %s, %s, %s, now(), 'deepseek')",
            )
            conn.commit()
            return cur.rowcount
    finally:
        conn.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", default="/Users/omega/Documents/GitHub/gaia/apps/classifier/deepseek/data/active_learning_round1.jsonl")
    parser.add_argument("--batch-size", type=int, default=50)
    parser.add_argument("--delay", type=float, default=30.0)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--output", type=str, default=None)
    args = parser.parse_args()

    output_path = args.output or args.input.replace(".jsonl", "_results.jsonl")

    # Load input
    with open(args.input) as f:
        all_items = [json.loads(line) for line in f if line.strip()]
    logger.info(f"Loaded {len(all_items)} items to label")

    # Load already done
    done_ihs = set()
    if args.resume and os.path.exists(output_path):
        with open(output_path) as f:
            for line in f:
                r = json.loads(line)
                done_ihs.add(r.get("infohash", "").lower())
        logger.info(f"Already done: {len(done_ihs)}")

    unlabeled = [item for item in all_items if item.get("infohash", "").lower() not in done_ihs]
    logger.info(f"Remaining: {len(unlabeled)}")

    if not unlabeled:
        logger.info("Nothing to do!")
        return

    pow_solver = ObscuraSolver(port=9222)
    client = DeepSeekClient(pow_solver=pow_solver)

    total_batches = (len(unlabeled) + args.batch_size - 1) // args.batch_size
    results_count = 0
    conversation_id = None
    consecutive_failures = 0

    with open(output_path, "a") as f:
        for batch_num in range(1, total_batches + 1):
            logger.info(f"--- Batch {batch_num}/{total_batches} ---")

            start_idx = (batch_num - 1) * args.batch_size
            end_idx = min(start_idx + args.batch_size, len(unlabeled))
            batch = unlabeled[start_idx:end_idx]

            prompt = build_prompt(batch)

            success = False
            for attempt in range(3):
                try:
                    reply = client.chat(prompt, conversation_id=conversation_id)
                    conversation_id = reply.conversation_id
                    logger.info(f"Got response ({len(reply.text)} chars)")

                    batch_results = parse_response(reply.text)

                    if not batch_results:
                        logger.warning(f"No results parsed (len={len(reply.text)})")
                        conversation_id = None
                        consecutive_failures += 1
                        if consecutive_failures >= 3:
                            logger.warning("3 consecutive failures, waiting 120s...")
                            time.sleep(120)
                            consecutive_failures = 0
                        break

                    consecutive_failures = 0
                    success = True

                    db_results = []
                    for r in batch_results:
                        ih = r.get("infohash", "").lower()
                        for s in batch:
                            if s.get("infohash", "").lower() == ih:
                                r["original_name"] = s.get("name", "")
                                r["predicted_category"] = s.get("predicted_category", "")
                                r["max_probability"] = s.get("max_probability", 0)
                                f.write(json.dumps(r) + "\n")
                                db_results.append(r)
                                results_count += 1
                                break

                    if db_results:
                        db_saved = save_to_database(db_results)
                        logger.info(f"Batch {batch_num}: {len(batch_results)} results, {db_saved} saved to DB (total: {results_count})")
                    else:
                        logger.info(f"Batch {batch_num}: {len(batch_results)} results (total: {results_count})")
                    f.flush()
                    break

                except Exception as e:
                    logger.error(f"Error: {type(e).__name__}: {e}")
                    consecutive_failures += 1
                    conversation_id = None
                    if consecutive_failures >= 5:
                        logger.warning("5 consecutive failures, waiting 300s...")
                        time.sleep(300)
                        consecutive_failures = 0

            if batch_num < total_batches and success:
                import random
                jittered_delay = args.delay * random.uniform(0.8, 1.2)
                time.sleep(jittered_delay)

    client.close()
    pow_solver.close()
    logger.info(f"Done! Total results: {results_count}")


if __name__ == "__main__":
    main()
