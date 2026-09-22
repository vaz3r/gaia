"""
Database connection pool for the health scorer.

Follows the pattern from apps/classifier/src/db.py with connection pooling,
keepalive settings, and safe connection release.
"""

from __future__ import annotations

import os
from typing import Any, Optional

import psycopg2
from psycopg2 import pool

DB_HOST = os.environ.get("DB_HOST", "localhost")
PG_PORT = int(os.environ.get("PG_PORT", 5432))
POSTGRES_USER = os.environ.get("POSTGRES_USER", "crawler")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "craw")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "")

_connection_pool: Optional[pool.ThreadedConnectionPool] = None


def get_pool() -> pool.ThreadedConnectionPool:
    global _connection_pool
    if _connection_pool is None or _connection_pool.closed:
        _connection_pool = pool.ThreadedConnectionPool(
            minconn=2,
            maxconn=10,
            host=DB_HOST,
            port=PG_PORT,
            user=POSTGRES_USER,
            password=PG_PASSWORD,
            dbname=POSTGRES_DB,
            options="-c statement_timeout=15000",
            keepalives=1,
            keepalives_idle=30,
            keepalives_interval=10,
            keepalives_count=5,
        )
    return _connection_pool


def release_conn(
    p: pool.ThreadedConnectionPool, conn: Any, close: bool = False
) -> None:
    """Safely roll back any uncommitted transaction and return connection to pool."""
    if conn and not getattr(conn, "closed", False):
        try:
            conn.rollback()
        except Exception:
            pass
        try:
            conn.autocommit = True
        except Exception:
            pass
    if conn:
        try:
            p.putconn(conn, close=close)
        except Exception:
            pass


def bytea_to_hex(val: Any) -> Optional[str]:
    if val is None:
        return None
    if isinstance(val, memoryview):
        return val.tobytes().hex()
    if isinstance(val, bytes):
        return val.hex()
    return str(val)
