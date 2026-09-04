#!/usr/bin/env python3
"""
Minimal DeepSeek-based torrent classifier.

Fetches unclassified torrents from PostgreSQL, sends them to DeepSeek for
classification, and records results in the same labeled_results table.

Usage:
    python classify.py              # classify one batch of 50
    python classify.py --loops 5    # classify 5 batches
    python classify.py --batch 100  # classify batches of 100
"""

import argparse
import json
import logging
import os
import random
import re
import ssl
import sys
import time
from pathlib import Path

import psycopg2
import psycopg2.extras

# Add parent dir so we can import the deepseek package
sys.path.insert(0, str(Path(__file__).resolve().parent))
from deepseek import DeepSeekClient, RateLimitError

# --- Logging ---
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
logger = logging.getLogger("classify")

# --- Database ---
DB_CONFIG = {
    "host": os.getenv("DB_HOST", "workspace-production"),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv(
        "DB_PASSWORD",
        "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b",
    ),
    "connect_timeout": 10,
}

CATEGORY_LABELS = [
    "Adult", "Anime", "Applications", "Documentaries",
    "Games", "Movies", "Music", "Television", "Other",
]

# Regex patterns for balanced extraction — bias batches toward underrepresented categories
CATEGORY_PATTERNS = {
    "Adult": r"(porn|xxx|adult|hentai|jav|onlyfans|brazzers|bangbros|nubile|naughty|teamSkeet|realitykings|mofos|caribbeancom|heyzo|1pondo|fc2|uncensored|fc2-ppv|erotic|massage|nude|naked)",
    "Anime": r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|Nekomoe|Tenshi)\]|(AT-X|Tokyo\s*MX|BS11|MBS|TBS|TV\s*Tokyo|KBS|Animax|Crunchyroll|Funimation|HIDIVE)",
    "Applications": r"(Adobe|Autodesk|JetBrains|Microsoft\s*Office|Windows\s*(10|11|Server)|VMware|MATLAB|Ableton|FL\s*Studio|Cubase|CorelDRAW|SolidWorks|Photoshop|Illustrator|Premiere|Acrobat|Kaspersky|Bitdefender|CCleaner|Acronis|EaseUS|Tenorshare|Office\s*20\d{2})",
    "Documentaries": r"(documentary|docuseries|frontline|NOVA|National\s*Geographic|Nat\s*Geo|Discovery\s*Channel|CuriosityStream|NHK|History\s*Channel|Panorama|Horizon|David\s*Attenborough|DW\s*Documentary|Storyville|Disneynature|Louis\s*Theroux)",
    "Games": r"(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|TiNYiSO|CPY|HOODLUM|RELOADED|DARKSiDERS|Goldberg|SteamRip|Steam-Rip|NSP|XCI|NSZ|CIA|VPK|WBFS|CSO|NDS|GBA)",
    "Movies": None,  # Fallback: random sampling
    "Music": r"(discography|album|soundtrack|OST|FLAC|lossless|320kbps|remastered|greatest\s*hits|compilation)",
    "Television": r"(S\d{1,2}E\d{1,3}|Season\s+\d+|Complete\s*Series|Episode\s+\d+)",
    "Other": None,  # Fallback: random sampling
}

SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS labeled_results (
    infohash bytea PRIMARY KEY,
    label_category text NOT NULL,
    confidence text,
    reason text,
    labeled_at timestamptz DEFAULT now(),
    source text DEFAULT 'deepseek'
);
"""

CLASSIFICATION_PROMPT = """\
You are a BitTorrent metadata classifier. Label each torrent with exactly one category.

## Categories

- **Adult** — Pornographic or sexual content (hentai, JAV, OnlyFans, explicit material)
- **Anime** — Japanese animation (fansub releases, anime series, OVAs)
- **Applications** — Software, tools, installers (Adobe, JetBrains, Office, etc.)
- **Documentaries** — Factual content (BBC, PBS, NatGeo, Discovery, etc.)
- **Games** — Video games (scene releases, console ROMs, Steam rips)
- **Movies** — Feature films (single file, title + year)
- **Music** — Audio content (albums, discographies, FLAC/MP3 releases)
- **Television** — Episodic TV series (seasons, episodes, talk shows)
- **Other** — Everything else (books, courses, spam, ambiguous content)

## Rules

1. Return ONLY a valid JSON array, no markdown fences, no explanation.
2. Each item must have exactly these keys: infohash, label_category, confidence, reason.
3. infohash must be the exact hex string from the input.
4. label_category must be one of: Adult, Anime, Applications, Documentaries, Games, Movies, Music, Television, Other
5. confidence must be one of: high, medium, low
6. reason must be 1 sentence, under 15 words.

## Torrents to classify

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


def _pick_target_category(cat_counts: dict) -> str:
    """Pick the category with the fewest labels to bias the next batch toward."""
    priority = ["Documentaries", "Other", "Games", "Applications", "Music",
                "Movies", "Anime", "Television", "Adult"]
    min_count = float("inf")
    target = priority[0]
    for cat in priority:
        cnt = cat_counts.get(cat, 0)
        if cnt < min_count:
            min_count = cnt
            target = cat
    return target


def fetch_unclassified_batch(limit: int) -> tuple[list[dict], str]:
    """Fetch unclassified torrents from PostgreSQL, biased toward target category."""
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(SCHEMA_SQL)

            # Get current category distribution
            cur.execute("SELECT label_category, COUNT(*) AS cnt FROM labeled_results GROUP BY label_category")
            cat_counts = {row["label_category"]: row["cnt"] for row in cur.fetchall()}

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

        torrents = []
        for row in rows:
            largest_files_raw = row["largest_files"] or []
            largest_files = []
            for lf in largest_files_raw[:3]:
                if isinstance(lf, dict):
                    largest_files.append({
                        "name": (lf.get("name") or "")[:80],
                        "size": lf.get("size", 0),
                    })

            torrents.append({
                "infohash": row["infohash"],
                "name": (row["name"] or "")[:200],
                "file_count": row["file_count"],
                "total_size_bytes": row["total_size"],
                "extensions": (row["extensions"] or [])[:5],
                "top_folders": (row["top_folders"] or [])[:5],
                "largest_files": largest_files,
            })

        logger.info(f"Fetched {len(torrents)} unclassified torrents (target: {target_category})")
        return torrents, target_category
    finally:
        conn.close()


def fetch_torrents_by_infohashes(infohashes: list[str]) -> list[dict]:
    """Fetch specific torrents by their infohashes."""
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            # Convert hex strings to bytea for comparison
            bytea_list = [bytes.fromhex(ih) for ih in infohashes]

            sql = f"""
            SELECT
                encode(t.infohash, 'hex') AS infohash,
                t.name,
                t.file_count,
                t.total_size,
                t.files AS files_raw
            FROM torrents t
            WHERE t.infohash = ANY(%s)
            """
            cur.execute(sql, (bytea_list,))
            rows = cur.fetchall()

        torrents = []
        for row in rows:
            torrents.append({
                "infohash": row["infohash"],
                "name": (row["name"] or "")[:200],
                "file_count": row["file_count"],
                "total_size": row["total_size"],
                "files_raw": row["files_raw"],
            })

        logger.info(f"Fetched {len(torrents)} torrents by infohash")
        return torrents
    finally:
        conn.close()


def build_prompt(torrents: list[dict]) -> str:
    """Build the classification prompt with torrent metadata."""
    prompt = CLASSIFICATION_PROMPT
    for i, t in enumerate(torrents, 1):
        prompt += f"{i}. infohash: {t['infohash']}\n"
        prompt += f"   name: {t['name']}\n"
        prompt += f"   file_count: {t['file_count']}\n"
        prompt += f"   total_size_bytes: {t.get('total_size_bytes', t.get('total_size', 0))}\n"
        if t.get("extensions"):
            prompt += f"   extensions: {', '.join(t['extensions'])}\n"
        if t.get("top_folders"):
            prompt += f"   top_folders: {', '.join(t['top_folders'])}\n"
        if t.get("largest_files"):
            lf_str = ", ".join(
                f"{f['name']} ({f['size']} bytes)" for f in t["largest_files"]
            )
            prompt += f"   largest_files: {lf_str}\n"
        prompt += "\n"
    return prompt


def parse_response(text: str) -> list[dict]:
    """Parse DeepSeek's JSON response into classification records."""
    # Strip markdown fences if present
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```(?:json)?\s*\n?", "", text)
        text = re.sub(r"\n?```\s*$", "", text)

    try:
        results = json.loads(text)
    except json.JSONDecodeError as e:
        logger.error(f"Failed to parse JSON response: {e}")
        logger.debug(f"Raw response: {text[:500]}")
        return []

    if not isinstance(results, list):
        logger.error(f"Expected JSON array, got {type(results).__name__}")
        return []

    return results


def validate_and_record(torrents: list[dict], results: list[dict]) -> dict:
    """Validate results and record valid ones in the database."""
    # Build a set of infohashes we sent for validation
    sent_infohashes = {t["infohash"].lower() for t in torrents}

    valid = []
    seen = set()
    skipped = 0
    duplicates = 0
    for r in results:
        # Check required fields
        ih = r.get("infohash", "").strip() if r.get("infohash") else ""
        cat = r.get("label_category", "")
        conf = r.get("confidence", "")
        reason = r.get("reason", "")

        if not ih or len(ih) != 40 or not all(c in "0123456789abcdefABCDEF" for c in ih):
            logger.warning(f"Skipping bad infohash: {ih[:20]}...")
            skipped += 1
            continue

        if cat not in CATEGORY_LABELS:
            logger.warning(f"Skipping invalid category '{cat}' for {ih[:16]}")
            skipped += 1
            continue

        ih_lower = ih.lower()
        if ih_lower in seen:
            duplicates += 1
            continue
        seen.add(ih_lower)
        valid.append((hex_to_bytea(ih), cat, conf, reason))

    if duplicates:
        logger.info(f"Deduplicated {duplicates} duplicate infohashes")

    if not valid:
        return {"recorded": 0, "skipped": skipped}

    conn = get_db()
    try:
        with conn.cursor() as cur:
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
                    source = 'deepseek'
                """,
                valid,
                template="(%s, %s, %s, %s, now(), 'deepseek')",
            )
            saved = cur.rowcount
        conn.commit()
        logger.info(f"Recorded {saved} classifications (skipped {skipped})")
        return {"recorded": saved, "skipped": skipped}
    except Exception as e:
        conn.rollback()
        logger.error(f"Database error: {e}")
        return {"recorded": 0, "skipped": skipped, "error": str(e)}
    finally:
        conn.close()


def main():
    parser = argparse.ArgumentParser(description="Classify torrents using DeepSeek")
    parser.add_argument("--batch", type=int, default=50, help="Batch size (default: 50)")
    parser.add_argument("--loops", type=int, default=1, help="Number of batches to process (default: 1)")
    parser.add_argument("--delay", type=float, default=10.0, help="Seconds between batches (default: 10)")
    parser.add_argument("--max-retries", type=int, default=5, help="Max retries on rate limit (default: 5)")
    parser.add_argument("--file", type=str, default=None, help="File with infohashes to classify (one per line)")
    args = parser.parse_args()

    ensure_schema()

    # Get total classified before starting
    conn = get_db()
    with conn.cursor() as cur:
        cur.execute("SELECT COUNT(*) FROM labeled_results")
        total_before = cur.fetchone()[0]
    conn.close()
    logger.info(f"Total classified before starting: {total_before}")

    # Initialize DeepSeek client
    logger.info("Initializing DeepSeek client...")
    client = DeepSeekClient()

    # Rate tracking
    request_times = []
    MAX_RPM = 10  # Max requests per minute

    # Circuit breaker: track failures in rolling window
    failure_times = []
    CIRCUIT_BREAKER_THRESHOLD = 0.25  # Stop if 25% of recent requests failed
    CIRCUIT_BREAKER_WINDOW = 300  # 5 minute rolling window
    CIRCUIT_BREAKER_COOLDOWN = 180  # 3 minute cooldown

    # Session reuse: store conversation_id to reuse across batches
    conversation_id = None

    # If --file is provided, load infohashes from file
    file_infohashes = []
    if args.file:
        with open(args.file) as f:
            file_infohashes = [line.strip() for line in f if line.strip()]
        logger.info(f"Loaded {len(file_infohashes)} infohashes from {args.file}")
        # Calculate loops from file size
        args.loops = (len(file_infohashes) + args.batch - 1) // args.batch
        logger.info(f"Will process {args.loops} batches of {args.batch}")

    for batch_num in range(1, args.loops + 1):
        logger.info(f"--- Batch {batch_num}/{args.loops} (size={args.batch}) ---")

        # Circuit breaker check
        now = time.time()
        failure_times = [t for t in failure_times if now - t < CIRCUIT_BREAKER_WINDOW]
        total_recent = len(request_times) + len(failure_times)
        if total_recent > 10:  # Only check after enough data
            failure_rate = len(failure_times) / total_recent
            if failure_rate > CIRCUIT_BREAKER_THRESHOLD:
                logger.warning(
                    f"Circuit breaker: {failure_rate:.1%} failure rate in last {CIRCUIT_BREAKER_WINDOW}s. "
                    f"Cooling down for {CIRCUIT_BREAKER_COOLDOWN}s..."
                )
                time.sleep(CIRCUIT_BREAKER_COOLDOWN)
                failure_times.clear()
                # Reset session after cooldown
                conversation_id = None
                continue

        # Rate limiting: ensure we don't exceed MAX_RPM
        now = time.time()
        request_times = [t for t in request_times if now - t < 60]
        if len(request_times) >= MAX_RPM:
            wait_time = 60 - (now - request_times[0]) + 1
            logger.info(f"Rate limit: waiting {wait_time:.1f}s (reached {MAX_RPM} RPM)")
            time.sleep(wait_time)

        # Fetch torrents — from file or random
        if file_infohashes:
            start_idx = (batch_num - 1) * args.batch
            end_idx = min(start_idx + args.batch, len(file_infohashes))
            batch_ihs = file_infohashes[start_idx:end_idx]
            torrents = fetch_torrents_by_infohashes(batch_ihs)
            target_category = "file-based"
        else:
            torrents, target_category = fetch_unclassified_batch(args.batch)
        if not torrents:
            logger.info("No more unclassified torrents. Done.")
            break

        # Build prompt with target category hint
        prompt = build_prompt(torrents)
        prompt += f"\nNote: This batch is biased toward **{target_category}** torrents. "
        prompt += "Pay extra attention to identifying torrents that match this category.\n"
        logger.info(f"Sending {len(torrents)} torrents to DeepSeek (target: {target_category})...")

        # Exponential backoff retry loop with jitter
        success = False
        for attempt in range(args.max_retries):
            try:
                reply = client.chat(
                    prompt,
                    conversation_id=conversation_id,
                    model="expert" if conversation_id is None else None,
                )
                conversation_id = reply.conversation_id  # Reuse for next batch
                request_times.append(time.time())
                logger.info(f"Got response ({len(reply.text)} chars)")
                success = True
                break
            except RateLimitError as e:
                # Use Retry-After from the response
                wait_time = max(e.retry_after, (2 ** attempt) * 10)
                wait_time += random.uniform(0, wait_time * 0.2)  # Add 20% jitter
                logger.warning(
                    f"Rate limited. Retrying in {wait_time:.1f}s "
                    f"(attempt {attempt + 1}/{args.max_retries}, Retry-After: {e.retry_after}s)"
                )
                failure_times.append(time.time())
                time.sleep(wait_time)
                # Reset conversation on rate limit (might be session-specific)
                conversation_id = None
            except (ssl.SSLError, ConnectionError, OSError) as e:
                # Transient network/SSL errors — retry with backoff
                wait_time = (2 ** attempt) * 15 + random.uniform(0, 10)
                logger.warning(
                    f"Network/SSL error: {e}. Retrying in {wait_time:.1f}s "
                    f"(attempt {attempt + 1}/{args.max_retries})"
                )
                failure_times.append(time.time())
                time.sleep(wait_time)
                # Reset conversation on network error
                conversation_id = None
            except Exception as e:
                error_str = str(e)
                is_rate_limit = (
                    "429" in error_str
                    or "rate" in error_str.lower()
                    or "too many" in error_str.lower()
                )
                if is_rate_limit and attempt < args.max_retries - 1:
                    wait_time = (2 ** attempt) * 10 + random.uniform(0, 5)
                    logger.warning(
                        f"Rate limited (string match). Retrying in {wait_time:.1f}s "
                        f"(attempt {attempt + 1}/{args.max_retries})"
                    )
                    failure_times.append(time.time())
                    time.sleep(wait_time)
                    conversation_id = None
                else:
                    logger.error(f"DeepSeek error: {e}")
                    failure_times.append(time.time())
                    break

        if not success:
            logger.error(f"Failed after {args.max_retries} attempts. Stopping.")
            break

        # Parse and record
        results = parse_response(reply.text)
        logger.info(f"Parsed {len(results)} classifications from response")

        record_result = validate_and_record(torrents, results)
        logger.info(
            f"Batch {batch_num}: recorded={record_result['recorded']}, "
            f"skipped={record_result['skipped']}"
        )

        # Delay between batches with jitter (80%-120% of base delay)
        if batch_num < args.loops:
            jittered_delay = args.delay * random.uniform(0.8, 1.2)
            logger.info(f"Waiting {jittered_delay:.1f}s before next batch...")
            time.sleep(jittered_delay)

    client.close()

    # Final count
    conn = get_db()
    with conn.cursor() as cur:
        cur.execute("SELECT COUNT(*) FROM labeled_results")
        total_after = cur.fetchone()[0]
    conn.close()
    logger.info(f"Done. Total classified: {total_before} -> {total_after} (+{total_after - total_before})")


if __name__ == "__main__":
    main()
