#!/usr/bin/env python3
"""
Selective Reclassification Script for Gaia Review Queue.
Reclassifies torrents where needs_review = true without touching accepted torrents
or overwriting labeled ground truth.
"""

import os
import sys
import time
import argparse
from pathlib import Path
from typing import List, Dict, Any

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(SRC_DIR))

import db
from classifier_service import TorrentClassifierService


def run_reclassification(batch_size: int = 2000, limit: int = None, dry_run: bool = False):
    print("=" * 80, flush=True)
    print("GAIA REVIEW QUEUE SELECTIVE RECLASSIFICATION", flush=True)
    print(f"Mode: {'DRY RUN (No updates)' if dry_run else 'PRODUCTION LIVE UPDATE'}", flush=True)
    print(f"Batch size: {batch_size:,} | Limit: {limit or 'ALL'}", flush=True)
    print("=" * 80, flush=True)

    # 1. Initialize classifier service (loads active model, e.g. v3)
    service = TorrentClassifierService.get_instance()
    model_version = service.active_version
    print(f"\n[1] Loaded active classifier model: {model_version}", flush=True)

    # 2. Count candidate review queue items (excluding ground truth)
    p = db.get_pool()
    conn = p.getconn()
    try:
        with conn.cursor() as cur:
            cur.execute("""
                SELECT count(*)
                FROM torrents t
                WHERE t.needs_review = true
                  AND NOT EXISTS (
                      SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash
                  );
            """)
            total_target = cur.fetchone()[0]
    finally:
        p.putconn(conn)

    print(f"[2] Total review queue torrents eligible for reclassification: {total_target:,}", flush=True)
    if total_target == 0:
        print("    Review queue is empty. Nothing to reclassify.")
        return

    target_to_process = min(total_target, limit) if limit else total_target
    print(f"    Will process: {target_to_process:,} torrents\n", flush=True)

    # 3. Stream and classify in batches using a server-side cursor
    conn = p.getconn()
    cursor_name = f"reclass_cur_{int(time.time()*1000)}"
    cur = conn.cursor(name=cursor_name)
    cur.itersize = batch_size

    try:
        query = """
            SELECT t.infohash, t.name, t.total_size, t.file_count, t.files, t.category
            FROM torrents t
            WHERE t.needs_review = true
              AND NOT EXISTS (
                  SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash
              )
        """
        if limit:
            query += f" LIMIT {int(limit)}"

        cur.execute(query)

        processed = 0
        accepted_count = 0
        still_flagged_count = 0
        category_shifts: Dict[str, int] = {}
        batch_items: List[Dict[str, Any]] = []
        old_categories: List[str] = []

        t_start = time.time()

        for r in cur:
            ih_bytes = r[0]
            ih_hex = db.bytea_to_hex(ih_bytes)
            name = r[1] or ""
            total_size = r[2] or 0
            file_count = r[3] or 1
            files = r[4]
            old_cat = r[5]

            if isinstance(files, str):
                try:
                    import json
                    files = json.loads(files)
                except Exception:
                    files = []
            if isinstance(files, list):
                files = files[:40]

            batch_items.append({
                "infohash": ih_hex,
                "infohash_bytes": ih_bytes,
                "name": name,
                "total_size": total_size,
                "file_count": file_count,
                "files": files or []
            })
            old_categories.append(old_cat)

            if len(batch_items) >= batch_size:
                acc, flag = process_batch(
                    service, batch_items, old_categories, category_shifts, dry_run=dry_run
                )
                processed += len(batch_items)
                accepted_count += acc
                still_flagged_count += flag
                rate = processed / max(time.time() - t_start, 0.1)
                print(
                    f"Processed {processed:,}/{target_to_process:,} "
                    f"({(processed/target_to_process)*100:.1f}%) | "
                    f"Accepted: {accepted_count:,} | Flagged: {still_flagged_count:,} | "
                    f"{rate:.1f} items/s",
                    flush=True
                )
                batch_items = []
                old_categories = []

        # Process any remaining items
        if batch_items:
            acc, flag = process_batch(
                service, batch_items, old_categories, category_shifts, dry_run=dry_run
            )
            processed += len(batch_items)
            accepted_count += acc
            still_flagged_count += flag
            rate = processed / max(time.time() - t_start, 0.1)
            print(
                f"Processed {processed:,}/{target_to_process:,} (100.0%) | "
                f"Accepted: {accepted_count:,} | Flagged: {still_flagged_count:,} | "
                f"{rate:.1f} items/s",
                flush=True
            )

        cur.close()
        conn.commit()

        total_time = time.time() - t_start
        print("\n" + "=" * 80, flush=True)
        print("RECLASSIFICATION SUMMARY", flush=True)
        print("=" * 80, flush=True)
        print(f"Model version applied:         {model_version}", flush=True)
        print(f"Total processed:               {processed:,}", flush=True)
        print(f"Accepted (drained from queue): {accepted_count:,} ({(accepted_count/max(processed, 1))*100:.2f}%)", flush=True)
        print(f"Remaining in review queue:     {still_flagged_count:,} ({(still_flagged_count/max(processed, 1))*100:.2f}%)", flush=True)
        print(f"Total time elapsed:            {total_time:.2f}s ({processed/max(total_time, 0.1):.1f} items/s)", flush=True)
        print("\nTop Category Assignments for Drained Items:", flush=True)
        for cat, cnt in sorted(category_shifts.items(), key=lambda x: x[1], reverse=True):
            print(f"  - {cat:<20}: {cnt:,}", flush=True)
        print("=" * 80, flush=True)

    finally:
        try:
            cur.close()
        except Exception:
            pass
        p.putconn(conn)


def process_batch(service, batch_items, old_categories, category_shifts, dry_run: bool = False):
    results = service.classify_batch(batch_items)
    update_records = []
    accepted = 0
    flagged = 0

    for item, res, old_cat in zip(batch_items, results, old_categories):
        cat = res["category"]
        conf = float(res["confidence"])
        needs_review = bool(res.get("needs_review", False))

        if not needs_review:
            accepted += 1
            category_shifts[cat] = category_shifts.get(cat, 0) + 1
        else:
            flagged += 1

        update_records.append({
            "infohash": item["infohash_bytes"],
            "category": cat,
            "category_confidence": conf,
            "needs_review": needs_review,
            "classified_at": "NOW()"
        })

    if not dry_run and update_records:
        bulk_update_classifications(update_records)

    return accepted, flagged


def bulk_update_classifications(records: List[Dict[str, Any]]):
    """Execute fast execute_values bulk update in PostgreSQL."""
    import psycopg2.extras
    p = db.get_pool()
    conn = p.getconn()
    try:
        with conn.cursor() as cur:
            query = """
                UPDATE torrents AS t
                SET 
                    category = v.category,
                    category_confidence = v.category_confidence,
                    needs_review = v.needs_review,
                    classified_at = now()
                FROM (VALUES %s) AS v(infohash, category, category_confidence, needs_review)
                WHERE t.infohash = v.infohash;
            """
            template = "(%s, %s, %s, %s)"
            vals = [
                (
                    r["infohash"],
                    r["category"],
                    r["category_confidence"],
                    r["needs_review"]
                )
                for r in records
            ]
            psycopg2.extras.execute_values(cur, query, vals, template=template, page_size=2000)
            conn.commit()
    finally:
        p.putconn(conn)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Selective Review Queue Reclassification")
    parser.add_argument("--batch-size", type=int, default=2000, help="Batch size (default: 2000)")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of items to reclassify")
    parser.add_argument("--dry-run", action="store_true", help="Perform inference without writing to DB")
    args = parser.parse_args()

    run_reclassification(batch_size=args.batch_size, limit=args.limit, dry_run=args.dry_run)
