#!/usr/bin/env python3
import sys
import os
import time
import signal
import argparse
from pathlib import Path
from typing import List, Dict, Any

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

import db
from classifier_service import TorrentClassifierService

_running = True

def sig_handler(signum, frame):
    global _running
    print("\n[!] Shutdown signal received. Finishing current batch...", flush=True)
    _running = False

signal.signal(signal.SIGINT, sig_handler)
signal.signal(signal.SIGTERM, sig_handler)


def run_worker(
    batch_size: int = 2000,
    poll_interval: int = 15,
    once: bool = False,
    dry_run: bool = False,
    limit: int = None
):
    print("=" * 80, flush=True)
    print("GAIA TORRENT CLASSIFIER: CONTINUOUS QUEUE WORKER", flush=True)
    print(f"Batch Size: {batch_size} | Poll Interval: {poll_interval}s | Dry Run: {dry_run} | Once: {once}", flush=True)
    print("=" * 80, flush=True)

    service = TorrentClassifierService.get_instance()
    total_processed = 0
    total_accepted = 0
    total_flagged = 0

    while _running:
        if limit and total_processed >= limit:
            print(f"\n[*] Reached requested limit ({limit:,} records). Exiting.", flush=True)
            break

        current_fetch = batch_size
        if limit and (total_processed + current_fetch) > limit:
            current_fetch = limit - total_processed

        service.check_reload()

        t0 = time.time()
        batch = db.fetch_unclassified_batch(limit=current_fetch)
        fetch_time = time.time() - t0

        if not batch:
            # Check if migration was applied
            metrics = db.get_queue_metrics()
            if not metrics.get("migration_applied"):
                print("[!] Waiting for schema migration 001_add_classification_columns.sql to be applied on 'torrents'...", flush=True)
            else:
                print(f"[*] Queue caught up (0 unclassified torrents). Sleeping {poll_interval}s...", flush=True)

            if once:
                break
            time.sleep(poll_interval)
            continue

        # Classify batch
        t1 = time.time()
        classifications = service.classify_batch(batch)
        infer_time = time.time() - t1

        update_records = []
        batch_accepted = 0
        batch_flagged = 0

        for i in range(len(batch)):
            item = batch[i]
            c = classifications[i]
            if c["needs_review"]:
                batch_flagged += 1
            else:
                batch_accepted += 1

            meta = {
                "margin": c["margin"],
                "review_type": c["review_type"],
                "top2": {"category": c["top2_category"], "confidence": c["top2_confidence"]},
                "model_version": c.get("meta", {}).get("model_version", "unknown")
            }

            update_records.append({
                "infohash": item["infohash"],
                "infohash_hex": item["infohash_hex"],
                "predicted_category": c["predicted_category"],
                "confidence": c["confidence"],
                "needs_review": c["needs_review"],
                "meta": meta
            })

        # Update DB
        t2 = time.time()
        if dry_run:
            updated_count = len(update_records)
            update_time = 0.0
        else:
            updated_count = db.bulk_update_classifications(update_records)
            update_time = time.time() - t2

        total_processed += len(batch)
        total_accepted += batch_accepted
        total_flagged += batch_flagged
        total_time = fetch_time + infer_time + update_time
        rate = len(batch) / max(total_time, 0.001)

        print(
            f"[{time.strftime('%H:%M:%S')}] Batch: {len(batch):>4} | "
            f"Rate: {rate:>6.0f} rec/s | "
            f"Accepted: {batch_accepted:>4} ({(batch_accepted/len(batch))*100:>5.1f}%) | "
            f"Flagged: {batch_flagged:>4} ({(batch_flagged/len(batch))*100:>5.1f}%) | "
            f"Timing: fetch {fetch_time:.2f}s, infer {infer_time:.2f}s, db {update_time:.2f}s "
            f"{'[DRY RUN]' if dry_run else ''}",
            flush=True
        )

        if once:
            break

    print(
        f"\nWorker Summary: Total Processed: {total_processed:,} | "
        f"Accepted: {total_accepted:,} | Flagged: {total_flagged:,}",
        flush=True
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Gaia Torrent Classifier Continuous Queue Worker")
    parser.add_argument("--batch-size", type=int, default=int(os.environ.get("WORKER_BATCH_SIZE", 2000)), help="Batch size")
    parser.add_argument("--poll-interval", type=int, default=int(os.environ.get("WORKER_POLL_INTERVAL", 15)), help="Sleep seconds when queue empty")
    parser.add_argument("--once", action="store_true", help="Process single batch and exit")
    parser.add_argument("--dry-run", action="store_true", help="Do not write updates to database")
    parser.add_argument("--limit", type=int, default=None, help="Stop after N total records")
    args = parser.parse_args()

    run_worker(
        batch_size=args.batch_size,
        poll_interval=args.poll_interval,
        once=args.once,
        dry_run=args.dry_run,
        limit=args.limit
    )
