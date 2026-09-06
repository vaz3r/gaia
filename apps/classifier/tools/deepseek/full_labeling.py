#!/usr/bin/env python3
"""
Full labeling of stratified samples.

Labels the remaining ~27K samples after seed batch.
Saves to both JSONL file AND PostgreSQL database.

Usage:
    python full_labeling.py                    # Label all remaining
    python full_labeling.py --limit 5000       # Label 5K for testing
    python full_labeling.py --resume           # Resume from last checkpoint
"""

import argparse
import json
import logging
import os
import random
import sys
import time
from pathlib import Path

import psycopg2
import psycopg2.extras

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
logger = logging.getLogger("full_labeling")

# Add parent dir so we can import the deepseek package
sys.path.insert(0, str(Path(__file__).resolve().parent))
from deepseek import DeepSeekClient, RateLimitError
from deepseek.pow_obscura import ObscuraSolver

DB_CONFIG = {
    "host": os.getenv("DB_HOST", "workspace-production"),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv(
        "DB_PASSWORD",
        "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b",
    ),
    "connect_timeout": 10,
}


def load_stratified_samples(path: Path) -> list[dict]:
    """Load all stratified samples."""
    samples = []
    with open(path) as f:
        for line in f:
            samples.append(json.loads(line))
    return samples


def load_labeled_infohashes(path: Path) -> set:
    """Load infohashes that have already been labeled."""
    labeled = set()
    if path.exists():
        with open(path) as f:
            for line in f:
                item = json.loads(line)
                labeled.add(item["infohash"].lower())
    return labeled


def build_prompt(torrents: list[dict]) -> str:
    """Build the classification prompt with torrent metadata."""
    from classify import CLASSIFICATION_PROMPT
    
    prompt = CLASSIFICATION_PROMPT
    for i, t in enumerate(torrents, 1):
        prompt += f"{i}. infohash: {t['infohash']}\n"
        prompt += f"   name: {t['name']}\n"
        prompt += f"   file_count: {t['file_count']}\n"
        prompt += f"   total_size_bytes: {t.get('total_size_bytes', 0)}\n"
        if t.get("extensions"):
            prompt += f"   extensions: {', '.join(t['extensions'])}\n"
        if t.get("top_folders"):
            prompt += f"   top_folders: {', '.join(t['top_folders'])}\n"
        if t.get("largest_files"):
            lf_str = ", ".join(
                f"{f['name']} ({f['size']} bytes)" for f in t["largest_files"]
            )
            prompt += f"   largest_files: {lf_str}\n"
        prompt += "\n"
    return prompt


def parse_response(text: str) -> list[dict]:
    """Parse DeepSeek's JSON response into classification records."""
    import re
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```(?:json)?\s*\n?", "", text)
        text = re.sub(r"\n?```\s*$", "", text)
    
    try:
        results = json.loads(text)
    except json.JSONDecodeError as e:
        logger.error(f"Failed to parse JSON response: {e}")
        logger.debug(f"Raw response: {text[:500]}")
        return []
    
    if not isinstance(results, list):
        logger.error(f"Expected JSON array, got {type(results).__name__}")
        return []
    
    return results


def save_to_database(results: list[dict]) -> int:
    """Save classification results to PostgreSQL database."""
    if not results:
        return 0
    
    conn = psycopg2.connect(**DB_CONFIG)
    try:
        with conn.cursor() as cur:
            # Build values for bulk insert
            values = []
            for r in results:
                ih = r.get("infohash", "").strip()
                cat = r.get("label_category", "")
                conf = r.get("confidence", "low")
                reason = r.get("reason", "")
                
                if not ih or len(ih) != 40:
                    continue
                if cat not in ["Adult", "Anime", "Applications", "Audiobooks", "Books & Learning", 
                              "Documentaries", "Games", "Movies", "Music", "Television", "Other"]:
                    continue
                
                # Convert hex to bytea
                try:
                    bytea_ih = bytes.fromhex(ih)
                except ValueError:
                    continue
                
                values.append((bytea_ih, cat, conf, reason))
            
            if not values:
                return 0
            
            # Bulk insert with ON CONFLICT update
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
            
            saved = cur.rowcount
        conn.commit()
        return saved
    except Exception as e:
        conn.rollback()
        logger.error(f"Database error: {e}")
        return 0
    finally:
        conn.close()


def main():
    parser = argparse.ArgumentParser(description="Full labeling of stratified samples")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of samples to label")
    parser.add_argument("--batch-size", type=int, default=50, help="Batch size for LLM (default: 50)")
    parser.add_argument("--delay", type=float, default=30.0, help="Seconds between batches (default: 30)")
    parser.add_argument("--resume", action="store_true", help="Resume from last checkpoint")
    parser.add_argument("--output", type=str, default=None, help="Output JSONL file")
    args = parser.parse_args()
    
    # Load samples
    samples_path = Path(__file__).parent / "data" / "stratified_samples.jsonl"
    if not samples_path.exists():
        logger.error(f"Samples file not found: {samples_path}")
        logger.error("Run stratified_sample.py first")
        sys.exit(1)
    
    all_samples = load_stratified_samples(samples_path)
    logger.info(f"Loaded {len(all_samples)} total samples")
    
    # Load already labeled
    output_path = Path(args.output) if args.output else Path(__file__).parent / "data" / "full_labeling_results.jsonl"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    
    labeled_infohashes = set()
    if args.resume and output_path.exists():
        labeled_infohashes = load_labeled_infohashes(output_path)
        logger.info(f"Resuming: {len(labeled_infohashes)} already labeled")
    
    # Filter unlabeled
    unlabeled = [s for s in all_samples if s["infohash"].lower() not in labeled_infohashes]
    logger.info(f"Unlabeled: {len(unlabeled)}")
    
    if args.limit:
        unlabeled = unlabeled[:args.limit]
        logger.info(f"Limited to {len(unlabeled)} samples")
    
    if not unlabeled:
        logger.info("No unlabeled samples remaining")
        return
    
    # Initialize DeepSeek client
    logger.info("Initializing DeepSeek client...")
    import subprocess
    obscura_proc = None
    try:
        import httpx as _httpx
        _httpx.get("http://127.0.0.1:9222/json/version", timeout=2)
        logger.info("Obscura already running on port 9222")
    except Exception:
        logger.info("Starting Obscura stealth browser...")
        obscura_proc = subprocess.Popen(
            ["obscura", "serve", "--stealth", "--port", "9222"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        time.sleep(3)
    
    pow_solver = ObscuraSolver(port=9222)
    client = DeepSeekClient(pow_solver=pow_solver)
    
    # Process in batches
    total_batches = (len(unlabeled) + args.batch_size - 1) // args.batch_size
    results_count = 0
    conversation_id = None  # Reuse conversation across batches
    consecutive_failures = 0
    
    # Open output file in append mode
    with open(output_path, "a") as f:
        for batch_num in range(1, total_batches + 1):
            logger.info(f"--- Batch {batch_num}/{total_batches} ---")
            
            start_idx = (batch_num - 1) * args.batch_size
            end_idx = min(start_idx + args.batch_size, len(unlabeled))
            batch = unlabeled[start_idx:end_idx]
            
            # Build prompt
            prompt = build_prompt(batch)
            
            # Call DeepSeek with retry logic
            success = False
            for attempt in range(3):
                try:
                    reply = client.chat(prompt, conversation_id=conversation_id)
                    conversation_id = reply.conversation_id  # Reuse for next batch
                    logger.info(f"Got response ({len(reply.text)} chars)")
                    
                    # Parse response
                    batch_results = parse_response(reply.text)
                    
                    if not batch_results:
                        logger.warning(f"No results parsed from response (len={len(reply.text)})")
                        # Start new conversation on empty response
                        conversation_id = None
                        consecutive_failures += 1
                        if consecutive_failures >= 3:
                            logger.warning("3 consecutive failures, waiting 120s...")
                            time.sleep(120)
                            consecutive_failures = 0
                        break
                    
                    consecutive_failures = 0
                    success = True
                    
                    # Merge with original data
                    db_results = []
                    for r in batch_results:
                        ih = r.get("infohash", "").lower()
                        for s in batch:
                            if s["infohash"].lower() == ih:
                                r["assigned_bucket"] = s.get("assigned_bucket", "Other")
                                r["original_name"] = s["name"]
                                f.write(json.dumps(r) + "\n")
                                db_results.append(r)
                                results_count += 1
                                break
                    
                    # Save to database
                    if db_results:
                        db_saved = save_to_database(db_results)
                        logger.info(f"Batch {batch_num}: {len(batch_results)} results, {db_saved} saved to DB (total: {results_count})")
                    else:
                        logger.info(f"Batch {batch_num}: {len(batch_results)} results (total: {results_count})")
                    f.flush()
                    break
                    
                except RateLimitError as e:
                    wait_time = max(e.retry_after, 120)  # At least 2 min cooldown
                    logger.warning(f"Rate limited. Waiting {wait_time}s... (attempt {attempt+1}/3)")
                    time.sleep(wait_time)
                    # Start new conversation after rate limit
                    conversation_id = None
                    consecutive_failures += 1
                except Exception as e:
                    logger.error(f"Error: {type(e).__name__}: {e}")
                    consecutive_failures += 1
                    # Start new conversation on error
                    conversation_id = None
                    if consecutive_failures >= 5:
                        logger.warning("5 consecutive failures, waiting 300s...")
                        time.sleep(300)
                        consecutive_failures = 0
            
            # Delay between batches with jitter
            if batch_num < total_batches and success:
                import random
                jittered_delay = args.delay * random.uniform(0.8, 1.2)
                time.sleep(jittered_delay)
    
    logger.info(f"Done. Total labeled: {results_count}")
    
    # Cleanup
    client.close()
    pow_solver.close()
    if obscura_proc:
        obscura_proc.terminate()
        obscura_proc.wait(timeout=5)


if __name__ == "__main__":
    main()
