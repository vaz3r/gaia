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
_CLASSIFIER_CAT_CACHE: Dict[str, Any] = {"ts": 0.0, "data": {}}

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
            options="-c statement_timeout=15000",
            keepalives=1,
            keepalives_idle=30,
            keepalives_interval=10,
            keepalives_count=5
        )
    return _connection_pool

def release_conn(p: pool.ThreadedConnectionPool, conn: Any, close: bool = False) -> None:
    """Safely roll back any uncommitted transaction and ensure autocommit mode before returning connection to pool."""
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
        conn.autocommit = True
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
        release_conn(p, conn)

def get_torrent_by_infohash(hex_infohash: str) -> Optional[Dict[str, Any]]:
    """Retrieve full details of a specific torrent by infohash."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            try:
                ih_bytes = hex_to_bytea(hex_infohash)
            except (ValueError, TypeError):
                return None
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
        release_conn(p, conn)

def fetch_training_data(min_confidence: str = 'medium', limit: Optional[int] = None) -> List[Dict[str, Any]]:
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
        if min_confidence == 'high':
            conf_cond = "l.confidence = 'high'"
        else:
            conf_cond = "l.confidence IN ('high', 'medium')"

        query = f"""
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
            WHERE {conf_cond} AND l.label_category != 'Other'
        """
        params = []
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

            # Eager feature extraction: compute text and dense vectors immediately to discard raw files JSON
            from feature_extractor import get_text_and_features
            clean_text, dense_vec = get_text_and_features({
                "name": name,
                "total_size": size,
                "file_count": count_val,
                "files": files,
            }, normalize_dense=True, dense_version=3)

            records.append({
                "infohash": ih,
                "label_category": label,
                "confidence": conf,
                "source": source,
                "name": name,
                "clean_text": clean_text,
                "dense_vector": dense_vec,
                "total_size": size,
                "file_count": count_val,
                "labeled_at": labeled_at,
                "integrity_score": integrity_score,
                "model_safe_probability": model_safe_probability,
                "metadata_quality_score": metadata_quality_score,
                "policy_action": policy_action
            })
            if count % 10000 == 0:
                print(f"      Loaded and extracted {count:,} records from database...", flush=True)
        cur.close()

        # Supplement Adult class if severely truncated by purge policies
        adult_count = sum(1 for r in records if r["label_category"] == "Adult")
        if adult_count < 5000:
            target_supplement = 7500 - adult_count
            print(f"      Supplementing Adult class ({adult_count:,} found in joined labels) with {target_supplement:,} verified accepted Adult torrents...", flush=True)
            supp_query = """
                SELECT 
                    t.infohash,
                    t.name,
                    t.total_size,
                    t.file_count,
                    t.files,
                    t.first_seen,
                    t.integrity_score,
                    t.model_safe_probability,
                    t.metadata_quality_score,
                    t.policy_action
                FROM torrents t
                WHERE t.category = 'Adult' 
                  AND t.needs_review = false 
                  AND t.category_confidence >= 0.75
                LIMIT %s
            """
            with conn.cursor() as supp_cur:
                supp_cur.execute(supp_query, (target_supplement,))
                for r in supp_cur:
                    ih = bytea_to_hex(r[0])
                    name = r[1]
                    size = r[2] or 0
                    count_val = r[3] or 1
                    files = r[4]
                    labeled_at = r[5].isoformat() if len(r) > 5 and r[5] else None
                    integrity_score = r[6] if len(r) > 6 and r[6] is not None else 100
                    model_safe_probability = r[7] if len(r) > 7 and r[7] is not None else 1.0
                    metadata_quality_score = r[8] if len(r) > 8 and r[8] is not None else 100
                    policy_action = str(r[9]) if len(r) > 9 and r[9] else 'ALLOW'

                    clean_text, dense_vec = get_text_and_features({
                        "name": name,
                        "total_size": size,
                        "file_count": count_val,
                        "files": files,
                    }, normalize_dense=True, dense_version=3)

                    records.append({
                        "infohash": ih,
                        "label_category": "Adult",
                        "confidence": "high",
                        "source": "verified_accepted_corpus",
                        "name": name,
                        "clean_text": clean_text,
                        "dense_vector": dense_vec,
                        "total_size": size,
                        "file_count": count_val,
                        "labeled_at": labeled_at,
                        "integrity_score": integrity_score,
                        "model_safe_probability": model_safe_probability,
                        "metadata_quality_score": metadata_quality_score,
                        "policy_action": policy_action
                    })

        conn.commit()
        return records
    finally:
        try:
            cur.close()
        except Exception:
            pass
        release_conn(p, conn)

def fetch_recent_canary_slice(limit: int = 1000) -> List[Dict[str, Any]]:
    """Fetch a slice of recent torrents for shadow/canary distribution testing."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            cur.execute("SET statement_timeout = 30000;")
            cur.execute(
                """
                SELECT infohash, name, total_size, file_count, files,
                       integrity_score, model_safe_probability, metadata_quality_score, policy_action
                FROM torrents
                WHERE verified_at IS NOT NULL
                ORDER BY verified_at DESC
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
        release_conn(p, conn)

def fetch_review_queue_slice(limit: int = 2000) -> List[Dict[str, Any]]:
    """Fetch torrents that currently require review (needs_review = true) for model validation."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            cur.execute("SET statement_timeout = 30000;")
            cur.execute(
                """
                SELECT infohash, name, total_size, file_count, files,
                       integrity_score, model_safe_probability, metadata_quality_score, policy_action, category
                FROM torrents
                WHERE needs_review = true
                ORDER BY popularity_score DESC
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
                    "policy_action": str(r[8]) if len(r) > 8 and r[8] else "ALLOW",
                    "current_category": r[9]
                })
            return items
    finally:
        release_conn(p, conn)

def fetch_unclassified_batch(limit: int = 2000) -> List[Dict[str, Any]]:
    """Fetch unclassified batch using FOR UPDATE SKIP LOCKED to prevent multi-worker collisions."""
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = True
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
                LIMIT %s;
            """
            cur.execute(query, (limit,))
            rows = cur.fetchall()
        conn.commit()

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
        return batch
    except Exception:
        try:
            conn.rollback()
        except Exception:
            pass
        raise
    finally:
        release_conn(p, conn)

_DISABLED_CAT_CACHE: Dict[str, Any] = {"ts": 0.0, "cats": set()}

def get_disabled_categories() -> set:
    """Fetch disabled categories from category_policies with 30s TTL cache."""
    global _DISABLED_CAT_CACHE
    now = time.time()
    if now - _DISABLED_CAT_CACHE["ts"] < 30.0:
        return _DISABLED_CAT_CACHE["cats"]
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = True
        with conn.cursor() as cur:
            cur.execute("SELECT category FROM category_policies WHERE is_enabled = false;")
            rows = cur.fetchall()
            conn.commit()
            disabled = {r[0] for r in rows}
            _DISABLED_CAT_CACHE = {"ts": now, "cats": disabled}
            return disabled
    except Exception:
        try:
            conn.rollback()
        except Exception:
            pass
        return _DISABLED_CAT_CACHE["cats"]
    finally:
        release_conn(p, conn)

def bulk_update_classifications(records: List[Dict[str, Any]], max_retries: int = 3) -> int:
    """Execute high-speed single-statement bulk update via UNNEST with deadlock retry.
    Automatically tombstones and purges torrents belonging to disabled categories."""
    if not records:
        return 0

    p = get_pool()

    # Intercept any records belonging to disabled categories
    disabled_cats = get_disabled_categories()
    if disabled_cats:
        allowed_records = []
        blocked_hashes = []
        blocked_cats = []
        for r in records:
            cat = r.get("predicted_category")
            needs_review = bool(r.get("needs_review", False))
            conf = float(r.get("confidence", 0.0))
            # Confidence protection: only auto-tombstone if high confidence (>= 85%) and not flagged for review.
            # Low confidence / ambiguous torrents are preserved in DB with needs_review=true so valid movies/games aren't lost.
            if cat in disabled_cats and (not needs_review) and conf >= 0.85:
                ih = r["infohash"] if isinstance(r["infohash"], (bytes, memoryview)) else hex_to_bytea(r["infohash_hex"])
                blocked_hashes.append(ih)
                blocked_cats.append(cat)
            else:
                allowed_records.append(r)

        if blocked_hashes:
            conn = p.getconn()
            try:
                with conn.cursor() as cur:
                    cur.execute("""
                        INSERT INTO blocked_infohashes (infohash, category, reason, blocked_at)
                        SELECT u.ih, u.cat, 'category_disabled', now()
                        FROM unnest(%s::bytea[], %s::text[]) AS u(ih, cat)
                        ON CONFLICT (infohash) DO NOTHING;
                    """, (blocked_hashes, blocked_cats))
                    cur.execute("DELETE FROM fetch_peer_outcomes WHERE infohash = ANY(%s);", (blocked_hashes,))
                    cur.execute("DELETE FROM verification_jobs WHERE infohash = ANY(%s);", (blocked_hashes,))
                    cur.execute("DELETE FROM infohash_sightings WHERE infohash = ANY(%s);", (blocked_hashes,))
                    cur.execute("DELETE FROM peer_torrents WHERE infohash = ANY(%s);", (blocked_hashes,))
                    cur.execute("DELETE FROM torrent_score_history WHERE infohash = ANY(%s);", (blocked_hashes,))
                    cur.execute("DELETE FROM torrents WHERE infohash = ANY(%s);", (blocked_hashes,))
                conn.commit()
            except Exception:
                try:
                    conn.rollback()
                except Exception:
                    pass
            finally:
                release_conn(p, conn)

        records = allowed_records
        if not records:
            return 0

    # Ensure deterministic B-tree lock ordering to prevent deadlocks and lock contention
    records_with_ih = []
    for r in records:
        ih = bytes(r["infohash"]) if isinstance(r["infohash"], (bytes, memoryview)) else hex_to_bytea(r["infohash_hex"])
        records_with_ih.append((ih, r))
    records_with_ih.sort(key=lambda x: x[0])

    total_updated = 0
    chunk_size = 100
    # Process in micro-chunks of 100 to minimize lock hold duration and contention with crawler
    for chunk_start in range(0, len(records_with_ih), chunk_size):
        chunk = records_with_ih[chunk_start : chunk_start + chunk_size]
        infohashes = [x[0] for x in chunk]
        categories = [x[1]["predicted_category"] for x in chunk]
        confidences = [float(x[1]["confidence"]) for x in chunk]
        needs_reviews = [bool(x[1]["needs_review"]) for x in chunk]
        metas = [json.dumps(x[1].get("meta", {})) for x in chunk]

        for attempt in range(max_retries):
            conn = p.getconn()
            try:
                conn.autocommit = False
                conn.set_session(readonly=False)
                with conn.cursor() as cur:
                    cur.execute("SET statement_timeout = '30000'; SET lock_timeout = '4000';")
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
                    total_updated += cur.rowcount
                    conn.commit()
                    break
            except Exception as e:
                try:
                    if conn and not conn.closed:
                        conn.rollback()
                except Exception:
                    pass
                err_str = str(e).lower()
                is_lock_or_timeout = any(term in err_str for term in [
                    "deadlock", "lock timeout", "lock_timeout", "statement timeout",
                    "canceling statement", "while locking tuple", "closed the connection",
                    "connection already closed"
                ])
                if is_lock_or_timeout and attempt < max_retries - 1:
                    time.sleep(0.3 * (attempt + 1))
                    continue
                if attempt == max_retries - 1:
                    # Fallback to single-item updates with fast lock_timeout so unblocked records succeed
                    for single_ih, single_cat, single_conf, single_nr, single_meta in zip(
                        infohashes, categories, confidences, needs_reviews, metas
                    ):
                        conn_single = p.getconn()
                        try:
                            conn_single.autocommit = True
                            with conn_single.cursor() as cur_single:
                                cur_single.execute("SET statement_timeout = '5000'; SET lock_timeout = '1000';")
                                cur_single.execute("""
                                    UPDATE torrents
                                    SET category = %s, category_confidence = %s, needs_review = %s,
                                        classified_at = now(), classification_meta = %s::jsonb
                                    WHERE infohash = %s;
                                """, (single_cat, single_conf, single_nr, single_meta, single_ih))
                                total_updated += cur_single.rowcount
                        except Exception:
                            pass
                        finally:
                            release_conn(p, conn_single)
                    break
                raise
            finally:
                release_conn(p, conn, close=conn.closed if conn else False)

    return total_updated

def upsert_label(
    infohash_hex: str,
    category: str,
    confidence: str = "high",
    reason: Optional[str] = None,
    source: str = "manual_web"
) -> Dict[str, Any]:
    """Upsert human or DeepSeek label into labeled_results.
    If category is 'Other', marks torrent as needs_review = false to drain review queue."""
    try:
        ih_bytes = hex_to_bytea(infohash_hex)
    except (ValueError, TypeError) as e:
        raise ValueError(f"Invalid infohash hex format: {infohash_hex}") from e
    p = get_pool()
    conn = p.getconn()
    try:
        conn.autocommit = False
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
        release_conn(p, conn)

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
        conn.autocommit = True
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

                # Category breakdown (cached in memory for 10 minutes)
                global _CLASSIFIER_CAT_CACHE
                now_ts = time.time()
                if now_ts - _CLASSIFIER_CAT_CACHE["ts"] < 600.0 and _CLASSIFIER_CAT_CACHE["data"]:
                    category_counts = _CLASSIFIER_CAT_CACHE["data"]
                else:
                    try:
                        cur.execute("SELECT category, count(*)::bigint FROM torrents WHERE category IS NOT NULL GROUP BY category ORDER BY count DESC;")
                        cat_rows = cur.fetchall()
                        category_counts = {r[0]: int(r[1]) for r in cat_rows}
                        _CLASSIFIER_CAT_CACHE = {"ts": now_ts, "data": category_counts}
                    except Exception:
                        if conn:
                            try:
                                conn.rollback()
                            except Exception:
                                pass
                        category_counts = _CLASSIFIER_CAT_CACHE.get("data", {})
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
        release_conn(p, conn)
