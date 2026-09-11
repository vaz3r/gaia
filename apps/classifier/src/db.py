import os
import time
import json
from typing import List, Dict, Any, Optional
import psycopg2
from psycopg2 import pool

DB_HOST = os.environ.get("DB_HOST", "workspace-production")
PG_PORT = int(os.environ.get("PG_PORT", 5432))
POSTGRES_USER = os.environ.get("POSTGRES_USER", "crawler")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "craw")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b")

_connection_pool: Optional[pool.ThreadedConnectionPool] = None

def get_pool() -> pool.ThreadedConnectionPool:
    global _connection_pool
    if _connection_pool is None or _connection_pool.closed:
        _connection_pool = pool.ThreadedConnectionPool(
            minconn=2,
            maxconn=25,
            host=DB_HOST,
            port=PG_PORT,
            user=POSTGRES_USER,
            password=PG_PASSWORD,
            dbname=POSTGRES_DB,
            options="-c statement_timeout=5000",
            keepalives=1,
            keepalives_idle=30,
            keepalives_interval=10,
            keepalives_count=5
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
    search: Optional[str] = None,
    needs_review: Optional[bool] = None,
    category: Optional[str] = None
) -> Dict[str, Any]:
    """Fetch paginated torrents from PostgreSQL with optional search, category, and review filtering."""
    p = get_pool()
    conn = p.getconn()
    try:
        with conn.cursor() as cur:
            if search and len(search.strip()) == 40:
                try:
                    ih_bytes = hex_to_bytea(search)
                    cur.execute(
                        """
                        SELECT infohash, name, total_size, file_count, (files IS NOT NULL) AS has_manifest, first_seen, last_seen, verified_at,
                               category, category_confidence, needs_review, classified_at
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
            elif needs_review is True:
                # Use cached metrics review queue count to avoid repetitive count queries
                metrics = get_queue_metrics()
                total = metrics.get("review_queue_depth", 0)

                # Order by popularity_score DESC to perfectly match idx_torrents_review_queue index scan
                cur.execute(
                    """
                    SELECT infohash, name, total_size, file_count, (files IS NOT NULL) AS has_manifest, first_seen, last_seen, verified_at,
                           category, category_confidence, needs_review, classified_at
                    FROM torrents
                    WHERE needs_review = TRUE
                    ORDER BY popularity_score DESC
                    OFFSET %s LIMIT %s;
                    """,
                    (offset, limit)
                )
                rows = cur.fetchall()
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
                    SELECT infohash, name, total_size, file_count, (files IS NOT NULL) AS has_manifest, first_seen, last_seen, verified_at,
                           category, category_confidence, needs_review, classified_at
                    FROM torrents
                    WHERE name ILIKE %s
                    ORDER BY verified_at DESC NULLS LAST
                    OFFSET %s LIMIT %s;
                    """,
                    (term, offset, limit)
                )
                rows = cur.fetchall()
            elif category and category.strip():
                cat = category.strip()
                cur.execute("SELECT count(*) FROM torrents WHERE category = %s;", (cat,))
                total = cur.fetchone()[0]

                cur.execute(
                    """
                    SELECT infohash, name, total_size, file_count, (files IS NOT NULL) AS has_manifest, first_seen, last_seen, verified_at,
                           category, category_confidence, needs_review, classified_at
                    FROM torrents
                    WHERE category = %s
                    ORDER BY verified_at DESC NULLS LAST
                    OFFSET %s LIMIT %s;
                    """,
                    (cat, offset, limit)
                )
                rows = cur.fetchall()
            else:
                # Query classified torrents (avoid displaying unclassified backlog in 'All Classified')
                # Reuse pre-calculated total_classified from metrics cache to avoid 3M row count(*) scan
                metrics = get_queue_metrics()
                total = metrics.get("total_classified", 0)

                cur.execute(
                    """
                    SELECT infohash, name, total_size, file_count, (files IS NOT NULL) AS has_manifest, first_seen, last_seen, verified_at,
                           category, category_confidence, needs_review, classified_at
                    FROM torrents
                    WHERE classified_at IS NOT NULL
                    ORDER BY classified_at DESC
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
                has_manifest = bool(r[4])
                first_seen = r[5].isoformat() if r[5] else None
                last_seen = r[6].isoformat() if r[6] else None
                verified_at = r[7].isoformat() if r[7] else None
                category = r[8] if len(r) > 8 else None
                confidence = float(r[9]) if len(r) > 9 and r[9] is not None else None
                n_review = bool(r[10]) if len(r) > 10 and r[10] is not None else False
                classified_at = r[11].isoformat() if len(r) > 11 and r[11] else None

                torrents.append({
                    "infohash": ih,
                    "name": name,
                    "total_size": total_size,
                    "total_size_formatted": format_size(total_size),
                    "file_count": file_count,
                    "first_seen": first_seen,
                    "last_seen": last_seen,
                    "verified_at": verified_at,
                    "has_files_manifest": has_manifest,
                    "category": category,
                    "category_confidence": confidence,
                    "needs_review": n_review,
                    "classified_at": classified_at
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
        with conn.cursor() as cur:
            ih_bytes = hex_to_bytea(hex_infohash)
            cur.execute(
                """
                SELECT infohash, name, total_size, file_count, files, first_seen, last_seen, verified_at,
                       category, category_confidence, needs_review, classified_at, classification_meta
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
            verified_at = row[7].isoformat() if row[7] else None
            category = row[8] if len(row) > 8 else None
            confidence = float(row[9]) if len(row) > 9 and row[9] is not None else None
            n_review = bool(row[10]) if len(row) > 10 and row[10] is not None else False
            classified_at = row[11].isoformat() if len(row) > 11 and row[11] else None
            classification_meta = row[12] if len(row) > 12 else None

            if isinstance(files, str):
                try:
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
                "last_seen": last_seen,
                "verified_at": verified_at,
                "category": category,
                "category_confidence": confidence,
                "needs_review": n_review,
                "classified_at": classified_at,
                "classification_meta": classification_meta
            }
    finally:
        p.putconn(conn)

def fetch_training_data(min_confidence: str = 'high', limit: Optional[int] = None) -> List[Dict[str, Any]]:
    """Query ground truth dataset directly from labeled_results joined with torrents.
    Excludes 'Other' to preserve the 10-class open-set boundary."""
    p = get_pool()
    conn = p.getconn()
    with conn.cursor() as init_cur:
        init_cur.execute("SET statement_timeout = 300000;")
    cursor_name = f"train_cur_{int(time.time()*1000)}"
    cur = conn.cursor(name=cursor_name)
    cur.itersize = 1500
    try:
        query = """
            SELECT 
                l.infohash,
                l.label_category,
                l.confidence,
                l.source,
                t.name,
                t.total_size,
                t.file_count,
                t.files,
                l.labeled_at,
                t.integrity_score,
                t.model_safe_probability,
                t.metadata_quality_score,
                t.policy_action
            FROM labeled_results l
            JOIN torrents t ON l.infohash = t.infohash
            WHERE l.confidence = %s AND l.label_category != 'Other'
        """
        params = [min_confidence]
        if limit:
            query += " LIMIT %s"
            params.append(limit)
        cur.execute(query, tuple(params))
        records = []
        count = 0
        for r in cur:
            count += 1
            ih = bytea_to_hex(r[0])
            label = r[1]
            conf = r[2]
            source = r[3]
            name = r[4]
            size = r[5] or 0
            count_val = r[6] or 1
            files = r[7]
            labeled_at = r[8].isoformat() if len(r) > 8 and r[8] else None
            integrity_score = r[9] if len(r) > 9 and r[9] is not None else 100
            model_safe_probability = r[10] if len(r) > 10 and r[10] is not None else 1.0
            metadata_quality_score = r[11] if len(r) > 11 and r[11] is not None else 100
            policy_action = str(r[12]) if len(r) > 12 and r[12] else 'ALLOW'

            if isinstance(files, str):
                try:
                    files = json.loads(files)
                except Exception:
                    files = []
            if isinstance(files, list):
                files = files[:100]

            records.append({
                "infohash": ih,
                "label_category": label,
                "confidence": conf,
                "source": source,
                "name": name,
                "total_size": size,
                "file_count": count_val,
                "files": files or [],
                "labeled_at": labeled_at,
                "integrity_score": integrity_score,
                "model_safe_probability": model_safe_probability,
                "metadata_quality_score": metadata_quality_score,
                "policy_action": policy_action
            })
            if count % 10000 == 0:
                print(f"      Loaded {count:,} records from database...", flush=True)
        cur.close()
        conn.commit()
        return records
    finally:
        try:
            cur.close()
        except Exception:
            pass
        p.putconn(conn)

def fetch_recent_canary_slice(limit: int = 1000) -> List[Dict[str, Any]]:
    """Fetch a slice of recent torrents for shadow/canary distribution testing."""
    p = get_pool()
    conn = p.getconn()
    try:
        with conn.cursor() as cur:
            cur.execute(
                """
                SELECT infohash, name, total_size, file_count, files,
                       integrity_score, model_safe_probability, metadata_quality_score, policy_action
                FROM torrents
                ORDER BY verified_at DESC NULLS LAST
                LIMIT %s;
                """,
                (limit,)
            )
            rows = cur.fetchall()
            items = []
            for r in rows:
                files = r[4]
                if isinstance(files, str):
                    try:
                        files = json.loads(files)
                    except Exception:
                        files = []
                if isinstance(files, list):
                    files = files[:100]
                items.append({
                    "infohash": bytea_to_hex(r[0]),
                    "name": r[1] or "",
                    "total_size": r[2] or 0,
                    "file_count": r[3] or 1,
                    "files": files or [],
                    "integrity_score": r[5] if len(r) > 5 and r[5] is not None else 100,
                    "model_safe_probability": r[6] if len(r) > 6 and r[6] is not None else 1.0,
                    "metadata_quality_score": r[7] if len(r) > 7 and r[7] is not None else 100,
                    "policy_action": str(r[8]) if len(r) > 8 and r[8] else "ALLOW"
                })
            return items
    finally:
        p.putconn(conn)

def fetch_unclassified_batch(limit: int = 2000) -> List[Dict[str, Any]]:
    """Fetch unclassified batch using FOR UPDATE SKIP LOCKED to prevent multi-worker collisions."""
    p = get_pool()
    conn = p.getconn()
    try:
        with conn.cursor() as cur:
            cur.execute("SET statement_timeout = 30000;")

            cur.execute("""
                SELECT 1 FROM information_schema.columns 
                WHERE table_name = 'torrents' AND column_name = 'classified_at';
            """)
            if not cur.fetchone():
                return []

            query = """
                SELECT infohash, name, total_size, file_count, files
                FROM torrents
                WHERE classified_at IS NULL
                ORDER BY verified_at DESC NULLS LAST
                LIMIT %s
                FOR UPDATE SKIP LOCKED;
            """
            cur.execute(query, (limit,))
            rows = cur.fetchall()

            batch = []
            for r in rows:
                files = r[4]
                if isinstance(files, str):
                    try:
                        files = json.loads(files)
                    except Exception:
                        files = []
                batch.append({
                    "infohash": r[0],
                    "infohash_hex": bytea_to_hex(r[0]),
                    "name": r[1] or "",
                    "total_size": r[2] or 0,
                    "file_count": r[3] or 1,
                    "files": files or []
                })
        conn.commit()
        return batch
    except Exception:
        conn.rollback()
        raise
    finally:
        p.putconn(conn)

def bulk_update_classifications(records: List[Dict[str, Any]], max_retries: int = 3) -> int:
    """Execute high-speed single-statement bulk update via UNNEST with deadlock retry."""
    if not records:
        return 0

    infohashes = []
    categories = []
    confidences = []
    needs_reviews = []
    metas = []

    for r in records:
        ih = r["infohash"] if isinstance(r["infohash"], (bytes, memoryview)) else hex_to_bytea(r["infohash_hex"])
        infohashes.append(ih)
        categories.append(r["predicted_category"])
        confidences.append(float(r["confidence"]))
        needs_reviews.append(bool(r["needs_review"]))
        metas.append(json.dumps(r.get("meta", {})))

    p = get_pool()

    for attempt in range(max_retries):
        conn = p.getconn()
        try:
            conn.set_session(readonly=False)
            with conn.cursor() as cur:
                cur.execute("SET statement_timeout = 30000;")
                query = """
                    UPDATE torrents AS t
                    SET 
                        category = c.category,
                        category_confidence = c.confidence,
                        needs_review = c.needs_review,
                        classified_at = now(),
                        classification_meta = c.meta::jsonb
                    FROM (
                        SELECT 
                            unnest(%s::bytea[]) AS infohash,
                            unnest(%s::text[]) AS category,
                            unnest(%s::real[]) AS confidence,
                            unnest(%s::boolean[]) AS needs_review,
                            unnest(%s::jsonb[]) AS meta
                    ) AS c
                    WHERE t.infohash = c.infohash;
                """
                cur.execute(query, (infohashes, categories, confidences, needs_reviews, metas))
                updated_count = cur.rowcount
                conn.commit()
                return updated_count
        except Exception as e:
            try:
                if conn and not conn.closed:
                    conn.rollback()
            except Exception:
                pass
            err_str = str(e).lower()
            if ("deadlock" in err_str or "closed the connection" in err_str or "connection already closed" in err_str) and attempt < max_retries - 1:
                time.sleep(1.0 * (attempt + 1))
                continue
            raise
        finally:
            if conn:
                try:
                    p.putconn(conn, close=conn.closed)
                except Exception:
                    pass

    return 0

def upsert_label(
    infohash_hex: str,
    category: str,
    confidence: str = "high",
    reason: Optional[str] = None,
    source: str = "manual_web"
) -> Dict[str, Any]:
    """Upsert human or DeepSeek label into labeled_results.
    If category is 'Other', marks torrent as needs_review = false to drain review queue."""
    ih_bytes = hex_to_bytea(infohash_hex)
    p = get_pool()
    conn = p.getconn()
    try:
        conn.set_session(readonly=False)
        with conn.cursor() as cur:
            # 1. Upsert into labeled_results
            cur.execute(
                """
                INSERT INTO labeled_results (infohash, label_category, confidence, reason, labeled_at, source)
                VALUES (%s, %s, %s, %s, now(), %s)
                ON CONFLICT (infohash) DO UPDATE SET
                    label_category = EXCLUDED.label_category,
                    confidence = EXCLUDED.confidence,
                    reason = EXCLUDED.reason,
                    labeled_at = now(),
                    source = EXCLUDED.source;
                """,
                (ih_bytes, category, confidence, reason, source)
            )

            # 2. Check if classification columns exist in torrents, update if so
            cur.execute("""
                SELECT 1 FROM information_schema.columns 
                WHERE table_name = 'torrents' AND column_name = 'category';
            """)
            if cur.fetchone():
                cur.execute(
                    """
                    UPDATE torrents
                    SET 
                        category = %s,
                        category_confidence = 1.0,
                        needs_review = false,
                        classified_at = now()
                    WHERE infohash = %s;
                    """,
                    (category, ih_bytes)
                )

            conn.commit()
            return {
                "infohash": infohash_hex,
                "label_category": category,
                "confidence": confidence,
                "source": source,
                "status": "upserted"
            }
    finally:
        p.putconn(conn)

_metrics_cache: Optional[Dict[str, Any]] = None
_metrics_cache_ts: float = 0.0

def get_queue_metrics() -> Dict[str, Any]:
    """Return live review queue and classification metrics with a 30s TTL cache."""
    global _metrics_cache, _metrics_cache_ts
    now = time.time()
    if _metrics_cache is not None and (now - _metrics_cache_ts) < 30.0:
        return _metrics_cache

    p = get_pool()
    conn = None
    try:
        conn = p.getconn()
        with conn.cursor() as cur:
            cur.execute("SELECT reltuples::bigint FROM pg_class WHERE relname = 'torrents';")
            total = cur.fetchone()[0] or 0

            # Check if columns exist
            cur.execute("""
                SELECT 1 FROM information_schema.columns 
                WHERE table_name = 'torrents' AND column_name = 'classified_at';
            """)
            cols_exist = bool(cur.fetchone())

            if cols_exist:
                cur.execute("SELECT count(*) FROM torrents WHERE classified_at IS NULL;")
                unclassified = cur.fetchone()[0]
                cur.execute("SELECT count(*) FROM torrents WHERE needs_review = true;")
                review_queue = cur.fetchone()[0]
                # High-speed index-only scan on idx_torrents_classified_at (<0.5ms)
                cur.execute("SELECT count(*) FROM torrents WHERE classified_at >= now() - interval '1 minute';")
                rate_1m_raw = cur.fetchone()[0] or 0
                cur.execute("SELECT count(*) FROM torrents WHERE classified_at >= now() - interval '5 minute';")
                rate_5m = cur.fetchone()[0] or 0
                # Smooth rate per minute using the 5m rolling window when between 2,000-item worker batches
                rate_1m = rate_1m_raw if rate_1m_raw > 0 else int(round(rate_5m / 5.0))

                # Category breakdown (instant lookup from pre-aggregated category_stats_summary table)
                try:
                    cur.execute("SELECT category, count FROM category_stats_summary ORDER BY count DESC;")
                    cat_rows = cur.fetchall()
                    category_counts = {r[0]: int(r[1]) for r in cat_rows}
                except Exception:
                    category_counts = {}
            else:
                unclassified = total
                review_queue = 0
                rate_1m = 0
                rate_5m = 0
                category_counts = {}

            cur.execute("SELECT count(*) FROM labeled_results;")
            total_labels = cur.fetchone()[0]

            total_classified = max(0, total - unclassified)
            classified_pct = round((total_classified / max(1, total)) * 100, 2)

            res = {
                "total_torrents": total,
                "total_classified": total_classified,
                "unclassified_torrents": unclassified,
                "classified_percentage": classified_pct,
                "review_queue_depth": review_queue,
                "rate_per_minute": rate_1m,
                "rate_5m": rate_5m,
                "category_counts": category_counts,
                "total_labeled_results": total_labels,
                "migration_applied": cols_exist
            }
            _metrics_cache = res
            _metrics_cache_ts = now
            return res
    except Exception as e:
        if _metrics_cache is not None:
            return _metrics_cache
        raise e
    finally:
        if conn:
            try:
                p.putconn(conn)
            except Exception:
                pass
