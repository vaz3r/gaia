import os
from typing import List, Dict, Any, Optional
import psycopg2
from psycopg2 import pool

DB_HOST = os.environ.get("DB_HOST", "workspace-production")
PG_PORT = int(os.environ.get("PG_PORT", 5432))
POSTGRES_USER = os.environ.get("POSTGRES_USER", "crawler")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "craw")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b")

_connection_pool: Optional[pool.SimpleConnectionPool] = None

def get_pool() -> pool.SimpleConnectionPool:
    global _connection_pool
    if _connection_pool is None or _connection_pool.closed:
        _connection_pool = pool.SimpleConnectionPool(
            minconn=1,
            maxconn=10,
            host=DB_HOST,
            port=PG_PORT,
            user=POSTGRES_USER,
            password=PG_PASSWORD,
            dbname=POSTGRES_DB
        )
    return _connection_pool

def hex_to_bytea(hex_str: str) -> bytes:
    cleaned = hex_str.strip().lower()
    if cleaned.startswith("0x"):
        cleaned = cleaned[2:]
    return bytes.fromhex(cleaned)

def bytea_to_hex(val: Any) -> Optional[str]:
    if val is None:
        return None
    if isinstance(val, memoryview):
        return val.tobytes().hex()
    if isinstance(val, bytes):
        return val.hex()
    return str(val)

def format_size(bytes_val: Optional[int]) -> str:
    if bytes_val is None or bytes_val <= 0:
        return "0 B"
    units = ["B", "KB", "MB", "GB", "TB"]
    val = float(bytes_val)
    idx = 0
    while val >= 1024.0 and idx < len(units) - 1:
        val /= 1024.0
        idx += 1
    return f"{val:.2f} {units[idx]}"

def get_torrents(
    offset: int = 0,
    limit: int = 25,
    search: Optional[str] = None
) -> Dict[str, Any]:
    """Fetch paginated torrents from PostgreSQL in strict READ_ONLY mode."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.set_session(readonly=True)
        with conn.cursor() as cur:
            if search and len(search.strip()) == 40:
                try:
                    ih_bytes = hex_to_bytea(search)
                    cur.execute(
                        """
                        SELECT infohash, name, total_size, file_count, files, first_seen, last_seen
                        FROM torrents
                        WHERE infohash = %s;
                        """,
                        (ih_bytes,)
                    )
                    rows = cur.fetchall()
                    total = len(rows)
                except Exception:
                    rows = []
                    total = 0
            elif search and search.strip():
                term = f"%{search.strip()}%"
                cur.execute(
                    """
                    SELECT count(*) FROM torrents WHERE name ILIKE %s;
                    """,
                    (term,)
                )
                total = cur.fetchone()[0]

                cur.execute(
                    """
                    SELECT infohash, name, total_size, file_count, files, first_seen, last_seen
                    FROM torrents
                    WHERE name ILIKE %s
                    ORDER BY first_seen DESC NULLS LAST
                    OFFSET %s LIMIT %s;
                    """,
                    (term, offset, limit)
                )
                rows = cur.fetchall()
            else:
                cur.execute("SELECT reltuples::bigint FROM pg_class WHERE relname = 'torrents';")
                approx_count = cur.fetchone()[0]
                total = max(approx_count, 0)

                cur.execute(
                    """
                    SELECT infohash, name, total_size, file_count, files, first_seen, last_seen
                    FROM torrents
                    ORDER BY first_seen DESC NULLS LAST
                    OFFSET %s LIMIT %s;
                    """,
                    (offset, limit)
                )
                rows = cur.fetchall()

            torrents = []
            for r in rows:
                ih = bytea_to_hex(r[0])
                name = r[1]
                total_size = r[2] or 0
                file_count = r[3] or 1
                files = r[4]
                first_seen = r[5].isoformat() if r[5] else None
                last_seen = r[6].isoformat() if r[6] else None

                torrents.append({
                    "infohash": ih,
                    "name": name,
                    "total_size": total_size,
                    "total_size_formatted": format_size(total_size),
                    "file_count": file_count,
                    "first_seen": first_seen,
                    "last_seen": last_seen,
                    "has_files_manifest": bool(files)
                })

            return {
                "torrents": torrents,
                "total": total,
                "offset": offset,
                "limit": limit
            }
    finally:
        p.putconn(conn)

def get_torrent_by_infohash(hex_infohash: str) -> Optional[Dict[str, Any]]:
    """Retrieve full details of a specific torrent by infohash."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.set_session(readonly=True)
        with conn.cursor() as cur:
            ih_bytes = hex_to_bytea(hex_infohash)
            cur.execute(
                """
                SELECT infohash, name, total_size, file_count, files, first_seen, last_seen
                FROM torrents
                WHERE infohash = %s;
                """,
                (ih_bytes,)
            )
            row = cur.fetchone()
            if not row:
                return None

            ih = bytea_to_hex(row[0])
            name = row[1]
            total_size = row[2] or 0
            file_count = row[3] or 1
            files = row[4]
            first_seen = row[5].isoformat() if row[5] else None
            last_seen = row[6].isoformat() if row[6] else None

            if isinstance(files, str):
                try:
                    import json
                    files = json.loads(files)
                except Exception:
                    files = []

            return {
                "infohash": ih,
                "name": name,
                "total_size": total_size,
                "total_size_formatted": format_size(total_size),
                "file_count": file_count,
                "files": files or [],
                "first_seen": first_seen,
                "last_seen": last_seen
            }
    finally:
        p.putconn(conn)
