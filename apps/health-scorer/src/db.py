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


def write_health_score(
    infohash: bytes,
    health_score: int,
    health_confidence: float,
    health_state: str,
    algorithm_version: str,
    evidence_source: str,
    direct_component: float = 0.0,
    seed_component: float = 0.0,
    peer_component: float = 0.0,
    dht_component: float = 0.0,
    failure_penalty: float = 0.0,
) -> bool:
    """Upsert a canonical health score into the health_scores table.

    This is the single writer for health scores from the scorer service.
    The crawler writes to torrents.health_score; the scorer writes here.
    Dashboard reads from health_scores via LEFT JOIN.

    The infohash is stored as a hex string (varchar(40)).
    Evidence details are packed into the evidence_summary JSONB column.

    Returns True on success, False on error.
    """
    import json as _json

    pool = get_pool()
    conn = pool.getconn()
    try:
        conn.autocommit = False
        ih_hex = infohash.hex() if isinstance(infohash, (bytes, memoryview)) else str(infohash)
        evidence_summary = _json.dumps({
            "evidence_source": evidence_source,
            "direct_component": round(direct_component, 4),
            "seed_component": round(seed_component, 4),
            "peer_component": round(peer_component, 4),
            "dht_component": round(dht_component, 4),
            "failure_penalty": round(failure_penalty, 4),
        })
        with conn.cursor() as cur:
            cur.execute(
                """
                INSERT INTO health_scores (
                    infohash, health_score, health_state, confidence,
                    evidence_summary, algorithm_version, health_calculated_at
                ) VALUES (
                    %s, %s, %s, %s, %s::jsonb, %s, now()
                )
                ON CONFLICT (infohash) DO UPDATE SET
                    health_score = EXCLUDED.health_score,
                    health_state = EXCLUDED.health_state,
                    confidence = EXCLUDED.confidence,
                    evidence_summary = EXCLUDED.evidence_summary,
                    algorithm_version = EXCLUDED.algorithm_version,
                    health_calculated_at = now()
                """,
                (
                    ih_hex,
                    health_score,
                    health_state,
                    health_confidence,
                    evidence_summary,
                    algorithm_version,
                ),
            )
        conn.commit()
        return True
    except Exception:
        try:
            conn.rollback()
        except Exception:
            pass
        return False
    finally:
        release_conn(pool, conn)
