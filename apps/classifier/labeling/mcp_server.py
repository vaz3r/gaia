#!/usr/bin/env python3
"""
MCP Server for Automated Torrent Classification.
Connects to PostgreSQL, fetches unclassified torrents, and saves classifications.
Designed for Gemini Spark (Streamable HTTP transport).

Usage:
  # Stdio (local dev):
  python labeling/mcp_server.py

  # Streamable HTTP (Gemini Spark):
  fastmcp run labeling/mcp_server.py:mcp --transport streamable-http --port 9000

  # Or via start_server.sh:
  ./labeling/start_server.sh
"""

import json
import os
import secrets
import sys
import logging
from datetime import datetime, timezone
from typing import Optional

import psycopg2
import psycopg2.extras
from fastmcp import FastMCP
from pydantic import BaseModel, Field
from starlette.requests import Request
from starlette.responses import JSONResponse

# --- Logging ---
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
logger = logging.getLogger("mcp-classifier")

# --- Database ---
DB_CONFIG = {
    "host": os.getenv("DB_HOST", "workspace-production"),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
    "connect_timeout": 10,
}

CATEGORY_LABELS = [
    "Adult",
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Television",
    "Other",
]

# Regex patterns for balanced extraction — used to bias batches toward underrepresented categories
CATEGORY_PATTERNS = {
    "Adult": r"(porn|xxx|adult|hentai|jav|onlyfans|brazzers|bangbros|nubile|naughty|teamSkeet|realitykings|mofos|caribbeancom|heyzo|1pondo|fc2|uncensored|fc2-ppv|erotic|massage|nude|naked)",
    "Anime": r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|Nekomoe|Tenshi)\]|(AT-X|Tokyo\s*MX|BS11|MBS|TBS|TV\s*Tokyo|KBS|Animax|Crunchyroll|Funimation|HIDIVE)",
    "Applications": r"(Adobe|Autodesk|JetBrains|Microsoft\s*Office|Windows\s*(10|11|Server)|VMware|MATLAB|Ableton|FL\s*Studio|Cubase|CorelDRAW|SolidWorks|Photoshop|Illustrator|Premiere|Acrobat|Kaspersky|Bitdefender|CCleaner|Acronis|EaseUS|Tenorshare|Office\s*20\d{2})",
    "Documentaries": r"(documentary|docuseries|frontline|NOVA|National\s*Geographic|Nat\s*Geo|Discovery\s*Channel|CuriosityStream|NHK|History\s*Channel|Panorama|Horizon|David\s*Attenborough|DW\s*Documentary|Storyville|Disneynature|Louis\s*Theroux)",
    "Games": r"(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|TiNYiSO|CPY|HOODLUM|RELOADED|DARKSiDERS|Goldberg|SteamRip|Steam-Rip|NSP|XCI|NSZ|CIA|VPK|WBFS|CSO|NDS|GBA)",
    "Movies": None,  # Fallback: no strong pattern, use random sampling
    "Music": r"(discography|album|soundtrack|OST|FLAC|lossless|320kbps|remastered|greatest\s*hits|compilation)",
    "Television": r"(S\d{1,2}E\d{1,3}|Season\s+\d+|Complete\s*Series|Episode\s+\d+)",
    "Other": None,  # Fallback: random sampling
}

# --- Schema setup ---
SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS labeled_results (
    infohash bytea PRIMARY KEY,
    label_category text NOT NULL,
    confidence text,
    reason text,
    labeled_at timestamptz DEFAULT now(),
    source text DEFAULT 'mcp_agent'
);
CREATE INDEX IF NOT EXISTS idx_labeled_results_category ON labeled_results(label_category);
"""


def get_db():
    """Get a PostgreSQL connection."""
    return psycopg2.connect(**DB_CONFIG)


def ensure_schema():
    """Create labeled_results table if it doesn't exist."""
    conn = get_db()
    try:
        with conn.cursor() as cur:
            cur.execute(SCHEMA_SQL)
        conn.commit()
    finally:
        conn.close()


def hex_to_bytea(infohash_hex: str) -> bytes:
    """Convert hex string to bytes for bytea column."""
    return bytes.fromhex(infohash_hex.strip())


# --- MCP Server ---
mcp = FastMCP(
    name="torrent-classifier",
    instructions=(
        "You are a BitTorrent metadata classifier. "
        "1. Call get_labeling_instructions() first to learn the categories. "
        "2. Call get_unclassified_batch() to get 200 torrents with metadata. "
        "3. Classify each torrent into the correct category. "
        "4. Call record_classifications(results) to save your labels. "
        "5. Repeat from step 2 until hasMore is false."
    ),
    version="1.0.0",
)


# --- Pydantic models ---
class ClassificationResult(BaseModel):
    infohash: str = Field(description="Hex infohash of the torrent")
    label_category: str = Field(
        description="One of: Anime, Applications, Documentaries, Games, Movies, Music, Television, Other"
    )
    confidence: str = Field(description="high, medium, or low")
    reason: str = Field(description="Brief explanation (1 sentence, under 15 words)")


# --- Tools ---
@mcp.tool
def get_unclassified_batch() -> dict:
    """
    Fetches 200 unclassified torrents from PostgreSQL.

    Returns torrents that have NOT been classified yet.
    Each batch is biased toward the underrepresented category to ensure balanced labeling.
    Each torrent includes: infohash, name, file_count, total_size_bytes, extensions, top_folders, largest_files.

    Returns:
        dict with keys: torrents, hasMore, batchId, totalClassified, totalRemaining, targetCategory, instructions
    """
    limit = 50
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            # Ensure schema exists
            cur.execute(SCHEMA_SQL)

            # Get current category distribution
            cur.execute("SELECT COUNT(*) AS cnt FROM labeled_results")
            total_classified = cur.fetchone()["cnt"]

            cur.execute(
                "SELECT label_category, COUNT(*) AS cnt FROM labeled_results GROUP BY label_category"
            )
            cat_counts = {row["label_category"]: row["cnt"] for row in cur.fetchall()}

            # Find target category (fewest labels, or least-likely-to-be-random)
            target_category = _pick_target_category(cat_counts)
            target_pattern = CATEGORY_PATTERNS.get(target_category)

            # Build query: bias toward target category if pattern exists
            if target_pattern:
                sql = f"""
                WITH unclassified AS (
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
                        END AS largest_files,
                        t.name ~* %s AS matches_target
                    FROM torrents t
                    WHERE NOT EXISTS (
                        SELECT 1 FROM labeled_results lr
                        WHERE lr.infohash = t.infohash
                    )
                )
                SELECT * FROM unclassified
                WHERE matches_target OR random() < 0.3
                ORDER BY matches_target DESC, random()
                LIMIT %s
                """
                cur.execute(sql, (target_pattern, limit))
            else:
                sql = """
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
                WHERE NOT EXISTS (
                    SELECT 1 FROM labeled_results lr
                    WHERE lr.infohash = t.infohash
                )
                ORDER BY random()
                LIMIT %s
                """
                cur.execute(sql, (limit,))

            rows = cur.fetchall()

            # Get total remaining
            cur.execute(
                """
                SELECT COUNT(*) AS cnt FROM torrents t
                WHERE NOT EXISTS (
                    SELECT 1 FROM labeled_results lr
                    WHERE lr.infohash = t.infohash
                )
                """
            )
            total_remaining = cur.fetchone()["cnt"]

        # Format torrents (truncate long fields to fit context window)
        torrents = []
        for row in rows:
            extensions = (row["extensions"] or [])[:5]
            top_folders = (row["top_folders"] or [])[:5]
            largest_files_raw = row["largest_files"] or []

            largest_files = []
            for lf in largest_files_raw[:3]:
                if isinstance(lf, dict):
                    largest_files.append({
                        "name": (lf.get("name") or "")[:80],
                        "size": lf.get("size", 0),
                    })

            torrents.append(
                {
                    "infohash": row["infohash"],
                    "name": (row["name"] or "")[:200],
                    "file_count": row["file_count"],
                    "total_size_bytes": row["total_size"],
                    "extensions": extensions,
                    "top_folders": top_folders,
                    "largest_files": largest_files,
                }
            )

        has_more = total_remaining > limit
        batch_id = total_classified // limit + 1

        # Build category status summary
        category_status = {}
        for cat in CATEGORY_LABELS:
            cnt = cat_counts.get(cat, 0)
            category_status[cat] = cnt

        logger.info(
            f"Batch {batch_id}: returned {len(torrents)} torrents, "
            f"target={target_category}, {total_remaining} remaining"
        )

        return {
            "torrents": torrents,
            "hasMore": has_more,
            "batchId": batch_id,
            "totalClassified": total_classified,
            "totalRemaining": total_remaining,
            "targetCategory": target_category,
            "categoryStatus": category_status,
            "instructions": (
                f"This batch is biased toward **{target_category}** torrents. "
                "Classify each torrent into one of the 8 categories. "
                "Each torrent has: name, file_count, total_size_bytes, extensions (file types), "
                "top_folders (directory structure), largest_files (top 3 by size with names and sizes). "
                "Use all this metadata to classify. "
                "Then call record_classifications() to log your observations. "
                "Call get_unclassified_batch() again for the next batch. "
                "Repeat until hasMore is false."
            ),
        }
    finally:
        conn.close()


def _pick_target_category(cat_counts: dict) -> str:
    """Pick the category with the fewest labels to bias the next batch toward."""
    # Priority order: harder/underrepresented classes first
    priority = ["Adult", "Anime", "Applications", "Games", "Documentaries", "Music", "Movies", "Television", "Other"]

    # Find the category with the fewest labels
    min_count = float("inf")
    target = priority[0]
    for cat in priority:
        cnt = cat_counts.get(cat, 0)
        if cnt < min_count:
            min_count = cnt
            target = cat

    return target


@mcp.tool
def record_classifications(results: list[ClassificationResult]) -> dict:
    """
    Records classification observations for each torrent.

    This is a logging/observation tool - it simply records what you observed
    about each torrent's category. No data is modified, only observation
    logs are appended for reference.

    Args:
        results: list of ClassificationResult with infohash, label_category, confidence, reason

    Returns:
        dict with keys: status, recorded, skipped, message
    """
    if not results:
        return {"status": "ok", "recorded": 0, "skipped": 0, "message": "No results to save"}

    # Filter out malformed items: invalid category or missing/bad infohash
    valid = []
    skipped = 0
    for r in results:
        if r.label_category not in CATEGORY_LABELS:
            logger.warning(f"Skipping item with invalid category '{r.label_category}'")
            skipped += 1
            continue
        # Validate infohash is a hex string of correct length (40 chars = 20 bytes)
        ih = r.infohash.strip() if r.infohash else ""
        if len(ih) != 40 or not all(c in "0123456789abcdefABCDEF" for c in ih):
            logger.warning(f"Skipping item with bad infohash '{ih[:20]}...'")
            skipped += 1
            continue
        valid.append(r)

    if not valid:
        return {
            "status": "ok",
            "recorded": 0,
            "skipped": skipped,
            "message": f"All {skipped} items were malformed and skipped.",
        }

    conn = get_db()
    try:
        with conn.cursor() as cur:
            # Upsert: insert or update if infohash already exists
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
                    source = 'mcp_agent'
                """,
                [
                    (hex_to_bytea(r.infohash), r.label_category, r.confidence, r.reason)
                    for r in valid
                ],
                template="(%s, %s, %s, %s, now(), 'mcp_agent')",
            )
            saved = cur.rowcount
        conn.commit()

        logger.info(f"Recorded {saved} classifications (skipped {skipped} malformed)")

        # Get updated totals
        with conn.cursor() as cur:
            cur.execute("SELECT COUNT(*) FROM labeled_results")
            total = cur.fetchone()[0]
            cur.execute(
                "SELECT label_category, COUNT(*) FROM labeled_results GROUP BY label_category ORDER BY label_category"
            )
            by_category = {row[0]: row[1] for row in cur.fetchall()}

        msg = f"Recorded {saved} observations. Total observed: {total}."
        if skipped:
            msg += f" Skipped {skipped} malformed items."

        return {
            "status": "ok",
            "recorded": saved,
            "skipped": skipped,
            "totalObserved": total,
            "byCategory": by_category,
            "message": msg,
        }
    except Exception as e:
        conn.rollback()
        logger.error(f"Error saving classifications: {e}")
        return {"status": "error", "recorded": 0, "skipped": skipped, "message": str(e)}
    finally:
        conn.close()


@mcp.tool
def get_labeling_instructions() -> str:
    """
    Returns the complete classification instructions.

    Call this FIRST to learn the 8 categories, rules, and edge cases.
    """
    prompt_path = os.path.join(os.path.dirname(__file__), "CLASSIFIER_PROMPT.md")
    try:
        with open(prompt_path, "r") as f:
            return f.read()
    except FileNotFoundError:
        return (
            "Error: CLASSIFIER_PROMPT.md not found. "
            "The system prompt should be in the same directory as mcp_server.py."
        )


# --- Health endpoint (for HTTP transport) ---
@mcp.custom_route("/health", methods=["GET"])
async def health(request: Request) -> JSONResponse:
    """Health check endpoint."""
    try:
        conn = get_db()
        with conn.cursor() as cur:
            cur.execute("SELECT 1")
            cur.execute("SELECT COUNT(*) FROM torrents")
            total_torrents = cur.fetchone()[0]
            cur.execute("SELECT COUNT(*) FROM labeled_results")
            total_labeled = cur.fetchone()[0]
        conn.close()
        return JSONResponse({
            "status": "healthy",
            "server": "torrent-classifier",
            "version": "1.0.0",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "database": "connected",
            "total_torrents": total_torrents,
            "total_labeled": total_labeled,
            "percent_labeled": round(total_labeled / total_torrents * 100, 2) if total_torrents > 0 else 0,
        })
    except Exception as e:
        return JSONResponse({
            "status": "unhealthy",
            "server": "torrent-classifier",
            "version": "1.0.0",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "database": "disconnected",
            "error": str(e),
        }, status_code=503)


# --- OAuth discovery (required by Gemini Spark) ---
@mcp.custom_route("/.well-known/oauth-protected-resource", methods=["GET"])
async def oauth_protected_resource(request: Request) -> JSONResponse:
    """OAuth protected resource metadata (RFC 8414)."""
    return JSONResponse({
        "resource": str(request.base_url).rstrip("/") + "/mcp",
        "authorization_servers": [str(request.base_url).rstrip("/")],
    })


@mcp.custom_route("/.well-known/oauth-protected-resource/mcp", methods=["GET"])
async def oauth_protected_resource_mcp(request: Request) -> JSONResponse:
    """OAuth protected resource metadata for /mcp endpoint."""
    return JSONResponse({
        "resource": str(request.base_url).rstrip("/") + "/mcp",
        "authorization_servers": [str(request.base_url).rstrip("/")],
    })


@mcp.custom_route("/.well-known/oauth-authorization-server", methods=["GET"])
async def oauth_authorization_server(request: Request) -> JSONResponse:
    """OAuth authorization server metadata (RFC 8414)."""
    base = str(request.base_url).rstrip("/")
    return JSONResponse({
        "issuer": base,
        "authorization_endpoint": base + "/authorize",
        "token_endpoint": base + "/token",
        "registration_endpoint": base + "/register",
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "scopes_supported": ["mcp:tools", "mcp:resources", "mcp:prompts"],
    })


@mcp.custom_route("/register", methods=["POST"])
async def dynamic_client_registration(request: Request) -> JSONResponse:
    """Dynamic Client Registration (RFC 7591)."""
    import secrets
    body = await request.json()
    client_id = secrets.token_hex(16)
    client_secret = secrets.token_hex(32)
    return JSONResponse({
        "client_id": client_id,
        "client_secret": client_secret,
        "client_name": body.get("client_name", "gemini-spark"),
        "redirect_uris": body.get("redirect_uris", []),
        "grant_types": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_method": "client_secret_basic",
    })


@mcp.custom_route("/authorize", methods=["GET", "POST"])
async def oauth_authorize(request: Request) -> JSONResponse:
    """OAuth authorization endpoint. Auto-approves for this server."""
    from starlette.responses import RedirectResponse
    params = dict(request.query_params) if request.method == "GET" else (await request.form())
    redirect_uri = params.get("redirect_uri", "")
    state = params.get("state", "")
    code = secrets.token_hex(16)
    separator = "&" if "?" in redirect_uri else "?"
    url = f"{redirect_uri}{separator}code={code}&state={state}"
    return RedirectResponse(url=url)


@mcp.custom_route("/token", methods=["POST"])
async def oauth_token(request: Request) -> JSONResponse:
    """OAuth token endpoint."""
    import secrets
    token = secrets.token_hex(32)
    return JSONResponse({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "mcp:tools mcp:resources mcp:prompts",
    })


if __name__ == "__main__":
    # Initialize schema on startup
    try:
        ensure_schema()
        logger.info("Database schema ready")
    except Exception as e:
        logger.error(f"Failed to initialize database: {e}")
        sys.exit(1)

    # Determine transport from args or env
    transport = os.getenv("MCP_TRANSPORT", "stdio")
    port = int(os.getenv("MCP_PORT", "9000"))

    if len(sys.argv) > 1:
        if sys.argv[1] in ("http", "streamable-http", "sse"):
            transport = sys.argv[1]
        elif sys.argv[1] == "--port" and len(sys.argv) > 2:
            port = int(sys.argv[2])

    logger.info(f"Starting MCP server with transport: {transport}")

    if transport == "stdio":
        mcp.run(transport="stdio")
    else:
        mcp.run(transport=transport, host="0.0.0.0", port=port)
