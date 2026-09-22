"""
Durable singleton cursor for the health scorer.

Uses a PostgreSQL advisory lock to ensure only one scorer instance processes
observations at a time. The cursor persists across restarts via the
health_scoring_cursor table.
"""

from __future__ import annotations

import logging
import time
from typing import Optional

import psycopg2

try:
    from . import db
except ImportError:
    import db

logger = logging.getLogger(__name__)

# Advisory lock key (arbitrary but unique to this service)
ADVISORY_LOCK_KEY = 0x4853434F  # "HSCO" in hex


class ScorerCursor:
    """Manages the durable high-water mark for observation processing.

    Ensures singleton operation via PostgreSQL advisory lock.
    """

    def __init__(self):
        self._conn: Optional[psycopg2.extensions.connection] = None
        self._lock_acquired = False

    def acquire(self) -> bool:
        """Acquire the singleton advisory lock.

        Returns True if this instance is the active scorer.
        Returns False if another instance holds the lock.
        """
        try:
            pool = db.get_pool()
            self._conn = pool.getconn()
            self._conn.autocommit = True
            with self._conn.cursor() as cur:
                # Try to acquire exclusive advisory lock (non-blocking)
                cur.execute("SELECT pg_try_advisory_lock(%s)", (ADVISORY_LOCK_KEY,))
                result = cur.fetchone()
                self._lock_acquired = result and result[0]
                if self._lock_acquired:
                    logger.info("Acquired scorer advisory lock (singleton active)")
                else:
                    logger.info("Another scorer instance holds the lock (standby)")
                return self._lock_acquired
        except Exception as e:
            logger.error("Failed to acquire advisory lock: %s", e)
            return False

    def release(self) -> None:
        """Release the advisory lock and return the connection."""
        if self._conn and not getattr(self._conn, "closed", False):
            try:
                with self._conn.cursor() as cur:
                    cur.execute("SELECT pg_advisory_unlock(%s)", (ADVISORY_LOCK_KEY,))
                logger.info("Released scorer advisory lock")
            except Exception as e:
                logger.error("Failed to release advisory lock: %s", e)
            finally:
                db.release_conn(db.get_pool(), self._conn)
                self._conn = None
                self._lock_acquired = False

    def get_cursor_position(self) -> tuple[int, float]:
        """Read the current cursor position from the database.

        Returns:
            (last_processed_id, last_processed_at_epoch)
        """
        pool = db.get_pool()
        conn = pool.getconn()
        try:
            conn.autocommit = True
            with conn.cursor() as cur:
                cur.execute(
                    "SELECT last_processed_id, "
                    "EXTRACT(EPOCH FROM last_processed_at)::bigint "
                    "FROM health_scoring_cursor WHERE id = 1"
                )
                row = cur.fetchone()
                if row:
                    return (row[0], row[1])
                return (0, 0)
        finally:
            db.release_conn(pool, conn)

    def advance(self, new_id: int) -> None:
        """Advance the cursor to a new position atomically.

        Only advances forward, never backward.
        """
        pool = db.get_pool()
        conn = pool.getconn()
        try:
            conn.autocommit = True
            with conn.cursor() as cur:
                cur.execute(
                    """
                    UPDATE health_scoring_cursor
                    SET last_processed_id = GREATEST(last_processed_id, %s),
                        last_processed_at = now(),
                        updated_at = now()
                    WHERE id = 1
                    """,
                    (new_id,),
                )
            logger.debug("Cursor advanced to id=%d", new_id)
        finally:
            db.release_conn(pool, conn)

    def get_batch_upper_bound(self) -> int:
        """Get the maximum observation id at the start of a batch.

        This snapshot is taken once per batch to ensure consistent processing.
        Rows inserted after this point will be picked up in the next batch.
        """
        pool = db.get_pool()
        conn = pool.getconn()
        try:
            conn.autocommit = True
            with conn.cursor() as cur:
                cur.execute("SELECT COALESCE(MAX(id), 0) FROM torrent_availability_observations")
                row = cur.fetchone()
                return row[0] if row else 0
        finally:
            db.release_conn(pool, conn)

    @property
    def is_active(self) -> bool:
        return self._lock_acquired

    def __enter__(self):
        self.acquire()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.release()
        return False
