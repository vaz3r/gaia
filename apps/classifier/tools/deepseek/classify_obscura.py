#!/usr/bin/env python3
"""
Obscura-based DeepSeek classifier.
Uses Obscura CDP server to bypass CloudFront and solve PoW in-browser.
"""

import argparse
import json
import logging
import os
import random
import sys
import time
from pathlib import Path

import httpx
import psycopg2
import psycopg2.extras

sys.path.insert(0, str(Path(__file__).resolve().parent))
from deepseek import RateLimitError

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
logger = logging.getLogger("classify_obscura")

DB_CONFIG = {
    "host": os.getenv("DB_HOST", "workspace-production"),
    "port": int(os.getenv("DB_PORT", "5432")),
    "user": os.getenv("DB_USER", "crawler"),
    "dbname": os.getenv("DB_NAME", "craw"),
    "password": os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
    "connect_timeout": 10,
}

CATEGORY_LABELS = [
    "Adult", "Anime", "Applications", "Documentaries",
    "Games", "Movies", "Music", "Television", "Other",
]

CATEGORY_PATTERNS = {
    "Adult": r"(porn|xxx|adult|hentai|jav|onlyfans|brazzers|bangbros|nubile|naughty|teamSkeet|realitykings|mofos|caribbeancom|heyzo|1pondo|fc2|uncensored|fc2-ppv|erotic|massage|nude|naked)",
    "Anime": r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|Nekomoe|Tenshi)\]|(AT-X|Tokyo\s*MX|BS11|MBS|TBS|TV\s*Tokyo|KBS|Animax|Crunchyroll|Funimation|HIDIVE)",
    "Applications": r"(Adobe|Autodesk|JetBrains|Microsoft\s*Office|Windows\s*(10|11|Server)|VMware|MATLAB|Ableton|FL\s*Studio|Cubase|CorelDRAW|SolidWorks|Photoshop|Illustrator|Premiere|Acrobat|Kaspersky|Bitdefender|CCleaner|Acronis|EaseUS|Tenorshare|Office\s*20\d{2})",
    "Documentaries": r"(documentary|docuseries|frontline|NOVA|National\s*Geographic|Nat\s*Geo|Discovery\s*Channel|CuriosityStream|NHK|History\s*Channel|Panorama|Horizon|David\s*Attenborough|DW\s*Documentary|Storyville|Disneynature|Louis\s*Theroux|BBC\s*Earth|Planet\s*Earth|Blue\s*Planet|Frozen\s*Planet|Life\s*on\s*Earth|Cosmos|Nature\s*of\s*Things|W5|The\s*National|Enquête|Envoyé|Vice|Vox)",
    "Games": r"(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|TiNYiSO|CPY|HOODLUM|RELOADED|DARKSiDERS|Goldberg|SteamRip|Steam-Rip|NSP|XCI|NSZ|CIA|VPK|WBFS|CSO|NDS|GBA)",
    "Movies": r"(BRRip|BDRip|BluRay|WEBRip|WEB-DL|HDRip|DVDRip|HDTV|1080p|720p|480p|x264|x265|HEVC|AAC|AC3|DTS|YIFY|YTS|RARBG|1337x|YTS\.MX)",
    "Music": r"(discography|album|soundtrack|OST|FLAC|lossless|320kbps|remastered|greatest\s*hits|compilation)",
    "Television": r"(S\d{1,2}E\d{1,3}|Season\s+\d+|Complete\s*Series|Episode\s+\d+)",
    "Other": r"(ebook|pdf|epub|mobi|udemy|coursera|tutorial|course|lecture|textbook|manual|guide|cookbook|recipes|self-help|self-help|meditation|yoga|fitness|workout|training|how-to|masterclass|skillshare|pluralsight|linux|ubuntu|debian|archlinux|centos|docker|kubernetes|aws|azure|gcp|github|gitlab|ansible|terraform|jenkins|ci/cd|devops)",
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

CLASSIFICATION_PROMPT = """You are a BitTorrent metadata classifier. Label each torrent with exactly one category.

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


def hex_to_bytea(infohash_hex: str) -> bytes:
    return bytes.fromhex(infohash_hex.strip())


def _pick_target_category(cat_counts: dict) -> str:
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


def fetch_unclassified_batch(limit: int, target_override: str = None) -> tuple:
    conn = get_db()
    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute(SCHEMA_SQL)
            cur.execute("SELECT label_category, COUNT(*) AS cnt FROM labeled_results GROUP BY label_category")
            cat_counts = {row["label_category"]: row["cnt"] for row in cur.fetchall()}

            if target_override and target_override in CATEGORY_PATTERNS:
                target_category = target_override
            else:
                target_category = _pick_target_category(cat_counts)
            target_pattern = CATEGORY_PATTERNS.get(target_category)

            if target_pattern:
                sql = f"""
                WITH unclassified AS (
                    SELECT
                        encode(t.infohash, 'hex') AS infohash,
                        t.name,
                        t.file_count,
                        t.total_size,
                        CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                            (SELECT array_agg(DISTINCT ext) FROM (
                                SELECT CASE WHEN jsonb_array_length(elem->'path') > 0 THEN
                                    lower(split_part(elem->'path'->>-1, '.', -1)) ELSE NULL END AS ext
                                FROM jsonb_array_elements(t.files) AS elem
                            ) sub WHERE ext IS NOT NULL AND ext != '' LIMIT 10)
                        ELSE NULL END AS extensions,
                        CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                            (SELECT array_agg(DISTINCT folder) FROM (
                                SELECT CASE WHEN jsonb_array_length(elem->'path') > 1 THEN
                                    elem->'path'->>0 ELSE NULL END AS folder
                                FROM jsonb_array_elements(t.files) AS elem
                            ) sub WHERE folder IS NOT NULL LIMIT 10)
                        ELSE NULL END AS top_folders,
                        CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                            (SELECT jsonb_agg(jsonb_build_object('name', sub.elem->'path'->>-1, 'size', sub.elem->'length'))
                             FROM (SELECT elem FROM jsonb_array_elements(t.files) AS elem
                                   ORDER BY (elem->'length')::bigint DESC LIMIT 3) sub)
                        ELSE NULL END AS largest_files,
                        t.name ~* %s AS matches_target
                    FROM torrents t
                    WHERE NOT EXISTS (SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash)
                )
                SELECT * FROM unclassified WHERE matches_target OR random() < 0.3
                ORDER BY matches_target DESC, random() LIMIT %s
                """
                cur.execute(sql, (target_pattern, limit))
            else:
                sql = """
                SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.file_count, t.total_size,
                    CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (SELECT array_agg(DISTINCT ext) FROM (
                            SELECT CASE WHEN jsonb_array_length(elem->'path') > 0 THEN
                                lower(split_part(elem->'path'->>-1, '.', -1)) ELSE NULL END AS ext
                            FROM jsonb_array_elements(t.files) AS elem
                        ) sub WHERE ext IS NOT NULL AND ext != '' LIMIT 10)
                    ELSE NULL END AS extensions,
                    CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (SELECT array_agg(DISTINCT folder) FROM (
                            SELECT CASE WHEN jsonb_array_length(elem->'path') > 1 THEN
                                elem->'path'->>0 ELSE NULL END AS folder
                            FROM jsonb_array_elements(t.files) AS elem
                        ) sub WHERE folder IS NOT NULL LIMIT 10)
                    ELSE NULL END AS top_folders,
                    CASE WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN
                        (SELECT jsonb_agg(jsonb_build_object('name', sub.elem->'path'->>-1, 'size', sub.elem->'length'))
                         FROM (SELECT elem FROM jsonb_array_elements(t.files) AS elem
                               ORDER BY (elem->'length')::bigint DESC LIMIT 3) sub)
                    ELSE NULL END AS largest_files
                FROM torrents t
                WHERE NOT EXISTS (SELECT 1 FROM labeled_results lr WHERE lr.infohash = t.infohash)
                ORDER BY random() LIMIT %s
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


def build_prompt(torrents: list) -> str:
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
            lf_str = ", ".join(f"{f['name']} ({f['size']} bytes)" for f in t["largest_files"])
            prompt += f"   largest_files: {lf_str}\n"
        prompt += "\n"
    return prompt


def parse_response(text: str) -> list:
    text = text.strip()
    if text.startswith("```"):
        import re
        text = re.sub(r"^```(?:json)?\s*\n?", "", text)
        text = re.sub(r"\n?```\s*$", "", text)
    try:
        results = json.loads(text)
    except json.JSONDecodeError as e:
        logger.error(f"Failed to parse JSON response: {e}")
        return []
    if not isinstance(results, list):
        logger.error(f"Expected JSON array, got {type(results).__name__}")
        return []
    return results


def validate_and_record(torrents: list, results: list) -> dict:
    sent_infohashes = {t["infohash"].lower() for t in torrents}
    valid = []
    seen = set()
    skipped = 0
    for r in results:
        ih = r.get("infohash", "").strip() if r.get("infohash") else ""
        cat = r.get("label_category", "")
        conf = r.get("confidence", "")
        reason = r.get("reason", "")

        if not ih or len(ih) != 40 or not all(c in "0123456789abcdefABCDEF" for c in ih):
            skipped += 1
            continue
        if cat not in CATEGORY_LABELS:
            skipped += 1
            continue
        ih_lower = ih.lower()
        if ih_lower in seen:
            continue
        seen.add(ih_lower)
        valid.append((hex_to_bytea(ih), cat, conf, reason))

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
    parser = argparse.ArgumentParser(description="Classify torrents using DeepSeek via Obscura")
    parser.add_argument("--batch", type=int, default=50)
    parser.add_argument("--loops", type=int, default=1)
    parser.add_argument("--delay", type=float, default=10.0)
    parser.add_argument("--target", type=str, default=None, choices=CATEGORY_LABELS)
    parser.add_argument("--cdp-url", type=str, default="http://127.0.0.1:9222")
    args = parser.parse_args()

    from playwright.sync_api import sync_playwright

    # Load session
    session_file = Path(__file__).resolve().parent / "session" / "session.json"
    with open(session_file) as f:
        session = json.load(f)
    token = session["token"]
    cookies = session["cookies"]

    with sync_playwright() as p:
        browser = p.chromium.connect_over_cdp(args.cdp_url)
        context = browser.contexts[0] if browser.contexts else browser.new_context()
        page = context.pages[0] if context.pages else context.new_page()

        # Inject cookies
        cookie_list = [{"name": n, "value": v, "domain": ".deepseek.com", "path": "/"} for n, v in cookies.items()]
        context.add_cookies(cookie_list)

        # Navigate to DeepSeek
        logger.info("Connecting to DeepSeek via Obscura...")
        try:
            page.goto("https://chat.deepseek.com/", wait_until="commit", timeout=30000)
        except:
            pass
        time.sleep(8)

        # Inject token
        try:
            page.evaluate(f'() => localStorage.setItem("userToken", JSON.stringify({{value: "{token}", __version: "0"}}))')
        except:
            pass
        time.sleep(2)

        # Verify connection
        try:
            check = page.evaluate('() => { try { return !!localStorage.getItem("userToken"); } catch(e) { return false; } }')
            if not check:
                logger.error("Failed to inject token into Obscura")
                return
        except Exception as e:
            logger.error(f"Connection check failed: {e}")
            return

        logger.info("Connected to DeepSeek via Obscura")

        # Load the JS PoW solver + chat function
        page.evaluate('''() => {
            window._dsChat = async function(prompt, model) {
                const token = JSON.parse(localStorage.getItem('userToken')).value;
                const headers = {'Content-Type': 'application/json', 'authorization': 'Bearer ' + token};
                
                // Create session
                const sR = await fetch('/api/v0/chat_session/create', {method:'POST', headers, body:'{}'});
                const sD = await sR.json();
                const sid = sD.data.biz_data.id;
                
                // Get PoW challenge
                const pR = await fetch('/api/v0/chat/create_pow_challenge', {method:'POST', headers, body: JSON.stringify({target_path: '/api/v0/chat/completion'})});
                const pD = await pR.json();
                const ch = pD.data.biz_data.challenge;
                
                // Solve PoW
                const prefix = ch.salt + '_' + ch.expire_at + '_';
                const challengeBytes = new TextEncoder().encode(ch.challenge);
                const prefixBytes = new TextEncoder().encode(prefix);
                
                let powHeader = null;
                for (let nonce = 0; nonce < 500000000; nonce++) {
                    const nonceBuf = new ArrayBuffer(8);
                    const dv = new DataView(nonceBuf);
                    dv.setUint32(0, nonce, true);
                    dv.setUint32(4, 0, true);
                    
                    const input = new Uint8Array(challengeBytes.length + prefixBytes.length + 8);
                    input.set(challengeBytes, 0);
                    input.set(prefixBytes, challengeBytes.length);
                    input.set(new Uint8Array(nonceBuf), challengeBytes.length + prefixBytes.length);
                    
                    const hashBuf = await crypto.subtle.digest('SHA-256', input);
                    const hashBytes = new Uint8Array(hashBuf);
                    const f64dv = new DataView(hashBytes.buffer, 0, 8);
                    const val = f64dv.getFloat64(0, true);
                    
                    if (val < ch.difficulty) {
                        const payload = {
                            algorithm: ch.algorithm,
                            challenge: ch.challenge,
                            salt: ch.salt,
                            answer: nonce,
                            signature: ch.signature,
                            target_path: ch.target_path,
                        };
                        powHeader = btoa(JSON.stringify(payload));
                        break;
                    }
                }
                
                if (!powHeader) return {error: 'PoW failed'};
                
                // Send completion
                const body = {
                    chat_session_id: sid,
                    parent_message_id: null,
                    prompt: prompt,
                    ref_file_ids: [],
                    thinking_enabled: false,
                    search_enabled: false,
                    action: null,
                    preempt: false,
                    model_type: model || 'default',
                };
                
                const cR = await fetch('/api/v0/chat/completion', {
                    method: 'POST',
                    headers: {...headers, 'x-ds-pow-response': powHeader},
                    body: JSON.stringify(body),
                });
                
                // Parse SSE stream
                const reader = cR.body.getReader();
                const decoder = new TextDecoder();
                let text = '';
                let activePath = null;
                let emittedInitial = false;
                
                while (true) {
                    const {done, value} = await reader.read();
                    if (done) break;
                    const chunk = decoder.decode(value);
                    for (const line of chunk.split('\\n')) {
                        if (!line.startsWith('data:')) continue;
                        const payload = line.slice(5).trim();
                        if (!payload || payload === '[DONE]') continue;
                        try {
                            const obj = JSON.parse(payload);
                            const v = obj.v;
                            if (typeof v === 'object' && v !== null && 'response' in v) {
                                for (const frag of v.response.fragments || []) {
                                    if (frag.type === 'RESPONSE' && frag.content) {
                                        activePath = 'response/fragments/-1/content';
                                        if (!emittedInitial) {
                                            emittedInitial = true;
                                            text += frag.content;
                                        }
                                    }
                                }
                            } else if ('p' in obj) {
                                activePath = obj.p;
                                if (obj.o === 'APPEND' && typeof v === 'string' && activePath.endsWith('content')) {
                                    text += v;
                                }
                            } else if (typeof v === 'string' && activePath && activePath.endsWith('content')) {
                                text += v;
                            }
                        } catch(e) {}
                    }
                }
                
                return {text, conversation_id: sid};
            };
        }''')
        logger.info("PoW solver loaded in browser")

        # Main loop
        for batch_num in range(1, args.loops + 1):
            logger.info(f"--- Batch {batch_num}/{args.loops} (size={args.batch}) ---")

            torrents, target_category = fetch_unclassified_batch(args.batch, target_override=args.target)
            if not torrents:
                logger.info("No more unclassified torrents. Done.")
                break

            prompt = build_prompt(torrents)
            prompt += f"\nNote: This batch is biased toward **{target_category}** torrents. "
            prompt += "Pay extra attention to identifying torrents that match this category.\n"

            logger.info(f"Sending {len(torrents)} torrents to DeepSeek (target: {target_category})...")

            try:
                result = page.evaluate(f'() => window._dsChat({json.dumps(prompt)}, "expert")')
                if not result or "error" in result:
                    logger.error(f"Chat error: {result}")
                    continue

                reply_text = result.get("text", "")
                logger.info(f"Got response ({len(reply_text)} chars)")

                results = parse_response(reply_text)
                logger.info(f"Parsed {len(results)} classifications")

                record_result = validate_and_record(torrents, results)
                logger.info(f"Batch {batch_num}: recorded={record_result['recorded']}, skipped={record_result['skipped']}")

            except Exception as e:
                logger.error(f"Batch error: {e}")
                continue

            if batch_num < args.loops:
                jittered_delay = args.delay * random.uniform(0.8, 1.2)
                logger.info(f"Waiting {jittered_delay:.1f}s before next batch...")
                time.sleep(jittered_delay)

        browser.close()

    # Final count
    conn = get_db()
    with conn.cursor() as cur:
        cur.execute("SELECT COUNT(*) FROM labeled_results")
        total = cur.fetchone()[0]
        cur.execute("SELECT label_category, COUNT(*) FROM labeled_results GROUP BY label_category ORDER BY COUNT(*) DESC")
        for row in cur.fetchall():
            logger.info(f"  {row[0]}: {row[1]}")
    conn.close()
    logger.info(f"Final total: {total}")


if __name__ == "__main__":
    main()
