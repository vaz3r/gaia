#!/usr/bin/env python3
"""
Health Scorer Continuous Queue Worker.

Follows the established pattern from apps/classifier/scripts/worker.py:
- Polling loop with batch processing
- Signal handling for graceful shutdown
- Heartbeat file for health checks
- CLI arguments for configuration
"""

import sys
import os
import time
import signal
import argparse
import logging
from pathlib import Path

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

from config import (
    WORKER_BATCH_SIZE,
    WORKER_POLL_INTERVAL,
    WORKER_ONCE,
    WORKER_DRY_RUN,
    HEARTBEAT_PATH,
    SHADOW_MODE,
    LEGACY_ADAPTER_ENABLED,
    LOG_LEVEL,
)
from cursor import ScorerCursor
from scorer import HealthScorer

# Configure logging
logging.basicConfig(
    level=getattr(logging, LOG_LEVEL.upper(), logging.INFO),
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("health_scorer.worker")

_running = True


def sig_handler(signum, frame):
    global _running
    print("\n[!] Shutdown signal received. Finishing current batch...", flush=True)
    _running = False


signal.signal(signal.SIGINT, sig_handler)
signal.signal(signal.SIGTERM, sig_handler)


def write_heartbeat():
    """Touch heartbeat file for health checks."""
    try:
        Path(HEARTBEAT_PATH).touch()
    except Exception:
        pass


def run_worker(
    batch_size: int = WORKER_BATCH_SIZE,
    poll_interval: int = WORKER_POLL_INTERVAL,
    once: bool = WORKER_ONCE,
    dry_run: bool = WORKER_DRY_RUN,
    limit: int = None,
    legacy_only: bool = False,
    legacy_offset: int = 0,
):
    print("=" * 80, flush=True)
    print("GAIA HEALTH SCORER: CONTINUOUS QUEUE WORKER", flush=True)
    print(
        f"Batch Size: {batch_size} | Poll Interval: {poll_interval}s | "
        f"Dry Run: {dry_run} | Once: {once} | Shadow: {SHADOW_MODE} | "
        f"Legacy Adapter: {LEGACY_ADAPTER_ENABLED} | Legacy Only: {legacy_only}",
        flush=True,
    )
    print("=" * 80, flush=True)

    # Acquire singleton lock
    cursor = ScorerCursor()
    if not cursor.acquire():
        print("[!] Another scorer instance is active. Waiting...", flush=True)
        if once:
            print("[!] --once specified but lock unavailable. Exiting.", flush=True)
            return
        # Wait and retry
        while _running:
            time.sleep(5)
            if cursor.acquire():
                break
        if not _running:
            return

    scorer = HealthScorer(shadow_mode=SHADOW_MODE)
    total_processed = 0

    try:
        while _running:
            if limit and total_processed >= limit:
                print(
                    f"\n[*] Reached requested limit ({limit:,} records). Exiting.",
                    flush=True,
                )
                break

            write_heartbeat()

            if legacy_only:
                # Process legacy records (shadow validation)
                count = scorer.process_legacy_batch(
                    batch_size=batch_size, offset=legacy_offset
                )
                legacy_offset += count
            else:
                # Process new observations (dual-source: falls back to legacy
                # internally when LEGACY_ADAPTER_ENABLED and observations empty)
                count = scorer.process_batch(batch_size=batch_size, cursor=cursor)

            total_processed += count or 0

            if count == 0:
                print(
                    f"[{time.strftime('%H:%M:%S')}] Queue caught up. "
                    f"Sleeping {poll_interval}s...",
                    flush=True,
                )
                if once:
                    break
                time.sleep(poll_interval)
            else:
                # Brief pause between active batches
                time.sleep(0.1)

    finally:
        cursor.release()

    stats = scorer.stats
    print(
        f"\nWorker Summary: Batches: {stats['batches_processed']:,} | "
        f"Observations: {stats['observations_processed']:,} | "
        f"Infohashes Scored: {stats['infohashes_scored']:,} | "
        f"Legacy Records: {stats['legacy_records_scored']:,} | "
        f"Canonical Writes: {stats['canonical_writes']:,}",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="GAIA Health Scorer Continuous Queue Worker"
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=int(os.environ.get("WORKER_BATCH_SIZE", 500)),
        help="Batch size for observation processing",
    )
    parser.add_argument(
        "--poll-interval",
        type=int,
        default=int(os.environ.get("WORKER_POLL_INTERVAL", 10)),
        help="Sleep seconds when queue empty",
    )
    parser.add_argument(
        "--once", action="store_true", help="Process single batch and exit"
    )
    parser.add_argument(
        "--dry-run", action="store_true", help="Do not write updates to database"
    )
    parser.add_argument(
        "--limit", type=int, default=None, help="Stop after N total records"
    )
    parser.add_argument(
        "--legacy-only",
        action="store_true",
        help="Process only legacy records (shadow validation)",
    )
    parser.add_argument(
        "--legacy-offset",
        type=int,
        default=0,
        help="Offset for legacy record processing",
    )
    args = parser.parse_args()

    run_worker(
        batch_size=args.batch_size,
        poll_interval=args.poll_interval,
        once=args.once,
        dry_run=args.dry_run,
        limit=args.limit,
        legacy_only=args.legacy_only,
        legacy_offset=args.legacy_offset,
    )
