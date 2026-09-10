#!/usr/bin/env python3
"""
MCP Server for OpenCode Automated Torrent Classification (macOS / Linux).
Connects to PostgreSQL, fetches review queue torrents with fast keyset pagination,
and saves classifications while automatically draining the review queue in real-time.
"""

import os
import sys
import json
import logging
from typing import Optional, List
from datetime import datetime, timezone

import psycopg2
import psycopg2.extras
from fastmcp import FastMCP
from pydantic import BaseModel, Field

# --- Logging ---
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
logger = logging.getLogger("mcp-classifier")

# --- Database Auto-Discovery ---
def resolve_db_host():
    explicit = os.getenv("DB_HOST")
    if explicit:
        return explicit
    import socket
    test_hosts = ["workspace-production", "100.87.194.112", "127.0.0.1"]
    for h in test_hosts:
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.settimeout(1.5)
            s.connect((h, 5432))
            s.close()
            logger.info(f"Connected to PostgreSQL at: {h}:5432")
            return h
        except Exception:
            continue
    return "workspace-production"

DB_CONFIG = {
    "host": resolve_db_host(),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
    "connect_timeout": 5,
}

# The canonical 10 production categories + fallback
CATEGORY_LABELS = [
    "Adult",
    "Anime",
    "Applications",
    "Audiobooks",
    "Books & Learning",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Television",
    "Other",
]

SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS labeled_results (
    infohash bytea PRIMARY KEY,
    label_category text NOT NULL,
    confidence text,
    reason text,
    labeled_at timestamptz DEFAULT now(),
    source text DEFAULT 'mcp_opencode'
);
CREATE INDEX IF NOT EXISTS idx_labeled_results_category ON labeled_results(label_category);
"""


def get_db():
    return psycopg2.connect(**DB_CONFIG)


def ensure_schema():
    conn = get_db()
    try:
        with conn.cursor() as cur:
            cur.execute(SCHEMA_SQL)
        conn.commit()
    finally:
        conn.close()


def hex_to_bytea(infohash_hex: str) -> bytes:
    return bytes.fromhex(infohash_hex.strip())


# --- FastMCP Server Setup ---
mcp = FastMCP(
    name="gaia-torrent-classifier",
    instructions=(
        "You are an expert BitTorrent metadata classifier helping drain the Gaia review queue. "
        "1. Call get_labeling_instructions() to learn the 10 categories, rules, and precedence hierarchy. "
        "2. Call get_review_queue_batch(limit=50) to fetch torrents flagged for review. "
        "3. Evaluate the name, total size, file count, extensions, and file paths. "
        "4. Assign each torrent to exactly one category with confidence (high/medium/low) and a brief reason. "
        "5. Call record_classifications(results) to save your labels and drain them from the review queue. "
        "6. Repeat until the review queue is empty."
    ),
    version="2.0.0",
)


class ClassificationItem(BaseModel):
    infohash: str = Field(description="40-character hex infohash of the torrent")
    label_category: str = Field(
        description="One of: Adult, Anime, Applications, Audiobooks, Books & Learning, Documentaries, Games, Movies, Music, Television, Other"
    )
    confidence: str = Field(description="Confidence level: high, medium, or low")
    reason: str = Field(description="Brief rationale (under 15 words)")


@mcp.tool
def get_labeling_instructions() -> str:
    """Returns detailed classification taxonomy rules, category boundaries, and examples."""
    prompt_file = os.path.join(os.path.dirname(__file__), "CLASSIFIER_PROMPT.md")
    if os.path.exists(prompt_file):
        with open(prompt_file, "r", encoding="utf-8") as f:
            return f.read()
    return """
## Categories & Precedence Hierarchy (Highest to Lowest):
1. Adult: Sexually explicit, pornographic, JAV, eromanga, hentai, OnlyFans. Overrides all other categories!
2. Anime: Japanese animation (fansubs, anime movies, OVAs, series).
3. Games: Video games (PC, console ROMs, repacks, ISOs, emulators).
4. Applications: Software programs, OS images, utilities, plugins.
5. Audiobooks: Spoken word books (M4B, chaptered MP3/FLAC, narrated content). Overrides Music!
6. Books & Learning: E-books (EPUB, PDF, CBR/CBZ comics), courses, tutorials, textbooks, sheet music.
7. Music: Musical audio releases, albums, discographies, soundtracks, FLAC/MP3 albums.
8. Television: Episodic TV series, broadcast specials, sports events, talk shows.
9. Movies: Feature-length cinema releases, live-action films.
10. Documentaries: Non-fiction factual, nature, history, science films/docuseries (BBC, PBS, NatGeo).
11. Other: Last resort when nothing else fits.
"""


@mcp.tool
def get_review_queue_batch(limit: int = 50, after_infohash: Optional[str] = None) -> dict:
    """
    Fetches a batch of torrents from the PostgreSQL Review Queue (needs_review = true).
    Torrents are ordered by infohash using an indexed keyset query (<30ms).
    Excludes any items already present in labeled_results.

    Args:
        limit: Number of items to fetch (default: 50, recommended: 20-50).
        after_infohash: Optional 40-char hex infohash to paginate to the next batch.

    Returns:
        dict with keys: torrents, remaining_in_queue, next_cursor, total_labeled
    """
    limit = max(1, min(limit, 100))
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(SCHEMA_SQL)

            cur.execute("SELECT count(*) FROM torrents WHERE needs_review = true;")
            total_queue = cur.fetchone()["count"]

            cur.execute("SELECT count(*) FROM labeled_results;")
            total_labeled = cur.fetchone()["count"]

            where_cursor = ""
            params = []
            if after_infohash and len(after_infohash) == 40:
                where_cursor = "AND t.infohash > %s"
                params.append(hex_to_bytea(after_infohash))
            params.append(limit)

            sql = f"""
            SELECT
                encode(t.infohash, 'hex') AS infohash,
                t.name,
                t.file_count,
                t.total_size,
                CASE
                    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (
                            SELECT array_agg(DISTINCT ext)
                            FROM (
                                SELECT
                                    CASE
                                        WHEN jsonb_array_length(elem->'path') > 0 THEN
                                            lower(split_part(elem->'path'->>-1, '.', -1))
                                        ELSE NULL
                                    END AS ext
                                FROM jsonb_array_elements(t.files) AS elem
                            ) sub
                            WHERE ext IS NOT NULL AND ext != ''
                            LIMIT 10
                        )
                    ELSE NULL
                END AS extensions,
                CASE
                    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (
                            SELECT array_agg(DISTINCT folder)
                            FROM (
                                SELECT
                                    CASE
                                        WHEN jsonb_array_length(elem->'path') > 1 THEN
                                            elem->'path'->>0
                                        ELSE NULL
                                    END AS folder
                                FROM jsonb_array_elements(t.files) AS elem
                            ) sub
                            WHERE folder IS NOT NULL
                            LIMIT 10
                        )
                    ELSE NULL
                END AS top_folders,
                CASE
                    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (
                            SELECT jsonb_agg(jsonb_build_object(
                                'name', sub.elem->'path'->>-1,
                                'size', sub.elem->'length'
                            ))
                            FROM (
                                SELECT elem
                                FROM jsonb_array_elements(t.files) AS elem
                                ORDER BY (elem->'length')::bigint DESC
                                LIMIT 3
                            ) sub
                        )
                    ELSE NULL
                END AS largest_files
            FROM torrents t
            WHERE t.needs_review = true
              {where_cursor}
              AND NOT EXISTS (
                  SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash
              )
            ORDER BY t.infohash
            LIMIT %s;
            """
            cur.execute(sql, tuple(params))
            rows = cur.fetchall()

        torrents = []
        for r in rows:
            largest_files_raw = r["largest_files"] or []
            largest_files = []
            for lf in largest_files_raw[:3]:
                if isinstance(lf, dict):
                    largest_files.append({
                        "name": (lf.get("name") or "")[:80],
                        "size": lf.get("size", 0),
                    })

            torrents.append({
                "infohash": r["infohash"],
                "name": (r["name"] or "")[:200],
                "file_count": r["file_count"],
                "total_size_bytes": r["total_size"],
                "extensions": (r["extensions"] or [])[:6],
                "top_folders": (r["top_folders"] or [])[:4],
                "largest_files": largest_files,
            })

        next_cursor = torrents[-1]["infohash"] if torrents else None

        return {
            "torrents": torrents,
            "fetched_count": len(torrents),
            "remaining_in_queue": total_queue,
            "total_labeled": total_labeled,
            "next_cursor": next_cursor,
        }
    finally:
        conn.close()


@mcp.tool
def record_classifications(results: List[ClassificationItem]) -> dict:
    """
    Saves verified classifications into PostgreSQL `labeled_results` AND directly updates `torrents`,
    immediately clearing `needs_review = false` to drain the Review Queue in real-time.

    Args:
        results: List of ClassificationItem objects with infohash, label_category, confidence, reason.

    Returns:
        dict with status, recorded count, and updated review queue totals.
    """
    if not results:
        return {"status": "ok", "recorded": 0, "message": "No items to save."}

    valid = []
    skipped = 0
    for r in results:
        cat = r.label_category.strip()
        if cat not in CATEGORY_LABELS:
            logger.warning(f"Skipping invalid category: '{cat}'")
            skipped += 1
            continue

        ih = r.infohash.strip().lower()
        if len(ih) != 40 or not all(c in "0123456789abcdef" for c in ih):
            logger.warning(f"Skipping invalid infohash: '{ih}'")
            skipped += 1
            continue

        valid.append((hex_to_bytea(ih), cat, r.confidence.strip().lower(), r.reason.strip()))

    if not valid:
        return {"status": "ok", "recorded": 0, "skipped": skipped, "message": "All items were malformed."}

    conn = get_db()
    try:
        with conn.cursor() as cur:
            # 1. Insert ground-truth labels
            psycopg2.extras.execute_values(
                cur,
                """
                INSERT INTO labeled_results (infohash, label_category, confidence, reason, labeled_at, source)
                VALUES %s
                ON CONFLICT (infohash) DO UPDATE SET
                    label_category = EXCLUDED.label_category,
                    confidence = EXCLUDED.confidence,
                    reason = EXCLUDED.reason,
                    labeled_at = now(),
                    source = 'mcp_opencode'
                """,
                valid,
                template="(%s, %s, %s, %s, now(), 'mcp_opencode')",
            )
            saved = cur.rowcount

            # 2. Update torrents table: set category, confidence, and clear needs_review
            update_data = [(cat, 0.95, bytea_ih) for (bytea_ih, cat, _conf, _reason) in valid]
            psycopg2.extras.execute_batch(
                cur,
                """
                UPDATE torrents
                SET category = %s,
                    category_confidence = %s,
                    needs_review = false
                WHERE infohash = %s
                """,
                update_data,
                page_size=100,
            )

            # 3. Retrieve updated queue metrics
            cur.execute("SELECT count(*) FROM torrents WHERE needs_review = true;")
            remaining_queue = cur.fetchone()[0]

            cur.execute("SELECT count(*) FROM labeled_results;")
            total_labeled = cur.fetchone()[0]

        conn.commit()
        logger.info(f"Successfully recorded {saved} labels and cleared from review queue! (Remaining: {remaining_queue:,})")

        return {
            "status": "success",
            "recorded": saved,
            "skipped": skipped,
            "remaining_in_queue": remaining_queue,
            "total_labeled": total_labeled,
            "message": f"Recorded {saved} items. Remaining in Review Queue: {remaining_queue:,}.",
        }
    except Exception as e:
        conn.rollback()
        logger.error(f"Failed to record classifications: {e}")
        return {"status": "error", "error": str(e)}
    finally:
        conn.close()


@mcp.tool
def get_queue_statistics() -> dict:
    """Returns live telemetry on review queue depth, ground-truth labels, and category distribution."""
    conn = get_db()
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT count(*) FROM torrents WHERE needs_review = true;")
            queue_depth = cur.fetchone()[0]

            cur.execute("SELECT count(*) FROM labeled_results;")
            total_labeled = cur.fetchone()[0]

            cur.execute("SELECT label_category, count(*) FROM labeled_results GROUP BY 1 ORDER BY 2 DESC;")
            breakdown = {row[0]: row[1] for row in cur.fetchall()}

        return {
            "review_queue_depth": queue_depth,
            "total_ground_truth_labels": total_labeled,
            "category_distribution": breakdown,
            "target_host": DB_CONFIG["host"],
        }
    finally:
        conn.close()


if __name__ == "__main__":
    ensure_schema()
    mcp.run(transport="stdio")
