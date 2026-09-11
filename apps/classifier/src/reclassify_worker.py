import os
import sys
import time
import json
import threading
from pathlib import Path
from typing import Dict, Any, Optional, List

import psycopg2.extras

import db
from classifier_service import TorrentClassifierService


class ReclassifyManager:
    _instance: Optional["ReclassifyManager"] = None
    _lock = threading.Lock()

    def __init__(self):
        self._thread: Optional[threading.Thread] = None
        self._cancel_requested = threading.Event()
        self._status: Dict[str, Any] = {
            "is_running": False,
            "model_version": None,
            "dry_run": False,
            "started_at": None,
            "total_target": 0,
            "processed": 0,
            "accepted": 0,
            "still_flagged": 0,
            "acceptance_rate": 0.0,
            "items_per_second": 0.0,
            "eta_seconds": None,
            "category_shifts": {},
            "last_error": None,
            "last_completed": None
        }

    @classmethod
    def get_instance(cls) -> "ReclassifyManager":
        with cls._lock:
            if cls._instance is None:
                cls._instance = ReclassifyManager()
            return cls._instance

    def get_status(self) -> Dict[str, Any]:
        with self._lock:
            return dict(self._status)

    def cancel(self) -> bool:
        with self._lock:
            if self._status["is_running"]:
                self._cancel_requested.set()
                return True
            return False

    def start(self, batch_size: int = 2000, limit: Optional[int] = None, dry_run: bool = False) -> Dict[str, Any]:
        with self._lock:
            if self._status["is_running"]:
                raise RuntimeError("Reclassification task is already running")

            service = TorrentClassifierService.get_instance()
            model_ver = service.metadata.get("version") or service.active_info.get("version", "unknown")

            # Count total eligible items in review queue
            try:
                metrics = db.get_queue_metrics()
                total_queue = metrics.get("review_queue_depth") or 100000
            except Exception:
                total_queue = 100000

            target_count = min(total_queue, limit) if limit else total_queue

            self._cancel_requested.clear()
            self._status = {
                "is_running": True,
                "model_version": model_ver,
                "dry_run": dry_run,
                "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "total_target": target_count,
                "processed": 0,
                "accepted": 0,
                "still_flagged": 0,
                "acceptance_rate": 0.0,
                "items_per_second": 0.0,
                "eta_seconds": None,
                "category_shifts": {},
                "last_error": None,
                "last_completed": None
            }

            self._thread = threading.Thread(
                target=self._run_worker,
                args=(batch_size, limit, dry_run, target_count),
                daemon=True
            )
            self._thread.start()

            return dict(self._status)

    def _run_worker(self, batch_size: int, limit: Optional[int], dry_run: bool, target_count: int):
        t_start = time.time()
        service = TorrentClassifierService.get_instance()

        try:
            processed = 0
            accepted_count = 0
            still_flagged_count = 0
            category_shifts: Dict[str, int] = {}
            last_infohash: Optional[bytes] = None

            while not self._cancel_requested.is_set():
                if limit and processed >= limit:
                    break

                current_batch_size = batch_size
                if limit and (processed + current_batch_size) > limit:
                    current_batch_size = limit - processed

                # Keyset pagination to prevent memory growth or long-lived named cursors
                p = db.get_pool()
                conn = p.getconn()
                batch_rows = []
                try:
                    with conn.cursor() as cur:
                        # Ensure batch worker has sufficient execution time
                        cur.execute("SET statement_timeout = '60000';")
                        if last_infohash is None:
                            query = """
                                SELECT t.infohash, t.name, t.total_size, t.file_count, t.files, t.category
                                FROM torrents t
                                WHERE t.needs_review = true
                                  AND NOT EXISTS (
                                      SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash
                                  )
                                ORDER BY t.infohash
                                LIMIT %s;
                            """
                            cur.execute(query, (current_batch_size,))
                        else:
                            query = """
                                SELECT t.infohash, t.name, t.total_size, t.file_count, t.files, t.category
                                FROM torrents t
                                WHERE t.needs_review = true
                                  AND t.infohash > %s
                                  AND NOT EXISTS (
                                      SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash
                                  )
                                ORDER BY t.infohash
                                LIMIT %s;
                            """
                            cur.execute(query, (last_infohash, current_batch_size))
                        batch_rows = cur.fetchall()
                finally:
                    p.putconn(conn)

                if not batch_rows:
                    break

                # Prepare items for classification
                batch_items: List[Dict[str, Any]] = []
                old_categories: List[str] = []

                for r in batch_rows:
                    ih_bytes = r[0]
                    last_infohash = ih_bytes
                    ih_hex = db.bytea_to_hex(ih_bytes)
                    name = r[1] or ""
                    total_size = r[2] or 0
                    file_count = r[3] or 1
                    files = r[4]
                    old_cat = r[5]

                    if isinstance(files, str):
                        try:
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

                t_batch = time.time()
                # Process batch through model
                acc, flag = self._process_batch(service, batch_items, old_categories, category_shifts, dry_run=dry_run)
                batch_dur = time.time() - t_batch
                processed += len(batch_items)
                accepted_count += acc
                still_flagged_count += flag
                self._update_progress(processed, accepted_count, still_flagged_count, category_shifts, t_start, target_count, batch_duration=batch_dur, batch_size=len(batch_items))

            with self._lock:
                total_elapsed = time.time() - t_start
                self._status["is_running"] = False
                self._status["last_completed"] = {
                    "completed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                    "cancelled": self._cancel_requested.is_set(),
                    "processed": processed,
                    "accepted": accepted_count,
                    "still_flagged": still_flagged_count,
                    "duration_seconds": round(total_elapsed, 1),
                    "items_per_second": round(processed / max(total_elapsed, 0.01), 1),
                    "category_shifts": dict(category_shifts)
                }

        except Exception as e:
            with self._lock:
                self._status["is_running"] = False
                self._status["last_error"] = str(e)

    def _update_progress(
        self,
        processed: int,
        accepted: int,
        flagged: int,
        shifts: Dict[str, int],
        t_start: float,
        target_count: int,
        batch_duration: float = 0.0,
        batch_size: int = 0
    ):
        elapsed = time.time() - t_start
        overall_rate = processed / max(elapsed, 0.01)
        if batch_duration > 0 and batch_size > 0:
            batch_rate = batch_size / max(batch_duration, 0.01)
            # Smooth blend: 75% recent batch rate, 25% overall rate
            rate = 0.75 * batch_rate + 0.25 * overall_rate
        else:
            rate = overall_rate

        remaining = max(0, target_count - processed)
        eta = (remaining / rate) if rate > 0 else None
        pct_accepted = (accepted / max(processed, 1)) * 100.0

        with self._lock:
            self._status["processed"] = processed
            self._status["accepted"] = accepted
            self._status["still_flagged"] = flagged
            self._status["acceptance_rate"] = round(pct_accepted, 2)
            self._status["items_per_second"] = round(rate, 1)
            self._status["eta_seconds"] = int(eta) if eta is not None else None
            self._status["category_shifts"] = dict(shifts)

    def _process_batch(
        self,
        service: TorrentClassifierService,
        batch_items: List[Dict[str, Any]],
        old_categories: List[str],
        category_shifts: Dict[str, int],
        dry_run: bool = False
    ):
        results = service.classify_batch(batch_items)
        update_records = []
        accepted = 0
        flagged = 0

        for item, res, old_cat in zip(batch_items, results, old_categories):
            cat = res["predicted_category"]
            conf = float(res["confidence"])
            needs_review = bool(res.get("needs_review", False))

            if not needs_review:
                accepted += 1
                category_shifts[cat] = category_shifts.get(cat, 0) + 1
            else:
                flagged += 1

            meta = res.get("meta", {})

            update_records.append({
                "infohash": item["infohash_bytes"],
                "infohash_hex": item["infohash"],
                "predicted_category": cat,
                "confidence": conf,
                "needs_review": needs_review,
                "meta": meta,
            })

        if not dry_run and update_records:
            db.bulk_update_classifications(update_records)

        return accepted, flagged
