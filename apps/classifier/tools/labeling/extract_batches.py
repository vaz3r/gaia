#!/usr/bin/env python3
"""
Extract targeted torrent batches from PostgreSQL for manual labeling.
Handles connection drops by reconnecting. Skips already-extracted batches.
"""
from __future__ import annotations

import json
import os
import re
import sys
import time
from pathlib import Path

import psycopg2

DB_CONFIG = {
    "host": os.environ.get("DB_HOST", "workspace-production"),
    "port": int(os.environ.get("PG_PORT", "5432")),
    "user": os.environ.get("POSTGRES_USER", "crawler"),
    "dbname": os.environ.get("POSTGRES_DB", "craw"),
    "password": os.environ.get("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
}

BATCH_SIZE = 100
OUTPUT_DIR = Path("labeling/batches")

SUB_QUERIES = {
    "applications_adobe": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* 'adobe'
          AND name !~* '(fitgirl|codex|subplease|erai-raws)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "applications_misc": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(autodesk|jetbrains|microsoft|office|windows (10|11|server)|vmware|matlab|ableton|fl studio|cubase|protools|coreldraw|solidworks|photoshop|illustrator|premiere|acrobat)'
          AND name !~* '(fitgirl|codex|subplease|erai-raws)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "games_scene": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|CPY|HOODLUM|RELOADED|DARKSiDERS|Goldberg|SteamRip)'
          AND name !~* '(adobe|autodesk|jetbrains|microsoft|office|photoshop|vmware|matlab)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "games_console": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(NSP|XCI|NSZ|CIA|VPK|WBFS|CSO|NDS|GBA|Switch ROM)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "documentaries": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(documentary|docuseries|frontline|NOVA|National Geographic|Nat Geo|Discovery Channel|CuriosityStream|NHK|History Channel|Panorama|Horizon|David Attenborough|DW Documentary|Storyville|Disneynature|Louis Theroux)'
          AND name !~* '(xxx|porn|sex|onlyfans|brazzers|hentai|jav|uncensored|fc2|caribbeancom|heyzo)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "music": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(discography|album|soundtrack|ost|flac|lossless|320kbps|remastered|greatest hits|compilation)'
          AND name !~* '(1080p|720p|2160p|4k|bluray|web-dl|x264|x265|fitgirl|codex|S[0-9]{{1,2}}E[0-9]{{1,3}})'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "movies": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE name ~* '(1080p|720p|2160p|4k|blu-ray|bluray|bdrip|web-dl|webrip|x264|x265|hevc|hdrip|dvdrip|remux)'
          AND name ~* '(19|20)[0-9][0-9]'
          AND name !~* '(S[0-9]{{1,2}}E[0-9]{{1,3}}|Season [0-9]+|fitgirl|codex|subplease|Erai-raws|bbc|pbs|nova|frontline|documentary)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
    "television": """
        SELECT encode(infohash, 'hex') as ih, name, file_count, total_size,
               COALESCE(
                   (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                   '[]'::jsonb
               ) as top_dirs
        FROM torrents
        WHERE (name ~* 'S[0-9]{{1,2}}E[0-9]{{1,3}}' OR name ~* 'Season [0-9]+' OR name ~* 'Complete Series')
          AND name !~* '(subplease|erai-raws|horriblesubs|fitgirl|codex|bbc|pbs|nova|frontline|national geographic|discovery channel|documentary|history channel)'
          AND infohash NOT IN (SELECT infohash FROM _exclude)
        ORDER BY RANDOM()
        LIMIT {limit}
    """,
}

BATCHES_PER_QUERY = {
    "applications_adobe": 20,
    "applications_misc": 30,
    "games_scene": 30,
    "games_console": 20,
    "documentaries": 50,
    "music": 50,
    "movies": 50,
    "television": 50,
}

CATEGORY_MAP = {
    "applications_adobe": "applications",
    "applications_misc": "applications",
    "games_scene": "games",
    "games_console": "games",
    "documentaries": "documentaries",
    "music": "music",
    "movies": "movies",
    "television": "television",
}


def load_existing_infohashes() -> set[bytes]:
    import glob
    existing = set()
    for f in glob.glob("data/**/*.jsonl", recursive=True):
        try:
            with open(f, encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        r = json.loads(line)
                        ih = r.get("infohash", "")
                        if ih:
                            hex_clean = re.sub(r"\\x", "", ih).lower()
                            if len(hex_clean) == 40:
                                existing.add(bytes.fromhex(hex_clean))
                    except (json.JSONDecodeError, ValueError):
                        continue
        except Exception:
            continue
    # Also load from already-extracted batches
    for batch_file in OUTPUT_DIR.rglob("batch_*.json"):
        try:
            with open(batch_file, encoding="utf-8") as fh:
                for item in json.load(fh):
                    ih = item.get("infohash", "")
                    if ih and len(ih) == 40:
                        existing.add(bytes.fromhex(ih))
        except Exception:
            continue
    return existing


def get_connection():
    return psycopg2.connect(**DB_CONFIG)


def setup_exclude_table(conn, exclude_ihs: set[bytes]):
    cur = conn.cursor()
    cur.execute("DROP TABLE IF EXISTS _exclude")
    cur.execute("CREATE TEMP TABLE _exclude (infohash bytea PRIMARY KEY)")
    ih_list = list(exclude_ihs)
    for i in range(0, len(ih_list), 5000):
        chunk = ih_list[i:i+5000]
        values_template = ",".join(["(%s)"] * len(chunk))
        cur.execute(f"INSERT INTO _exclude VALUES {values_template}", chunk)
    conn.commit()
    return cur


def flatten_files(raw_files) -> list[str]:
    if not raw_files:
        return []
    out = []
    for item in raw_files:
        if isinstance(item, list):
            for x in item:
                if isinstance(x, str):
                    out.append(x)
        elif isinstance(item, str):
            out.append(item)
    return out


def extract_batches(category: str, query_template: str, n_batches: int) -> int:
    batch_dir = OUTPUT_DIR / CATEGORY_MAP[category]
    batch_dir.mkdir(parents=True, exist_ok=True)

    existing = list(batch_dir.glob("batch_*.json"))
    start_num = len(existing) + 1

    total_extracted = 0
    conn = None
    cur = None

    for batch_idx in range(n_batches):
        batch_num = start_num + batch_idx
        batch_file = batch_dir / f"batch_{batch_num:03d}.json"

        if batch_file.exists():
            print(f"  [skip] {batch_file.name} already exists", flush=True)
            # Add these hashes to exclusion on next connection
            continue

        # Ensure connection is alive
        try:
            if conn is None or conn.closed:
                conn = get_connection()
                cur = setup_exclude_table(conn, set())
                # Reload exclusion from all existing batches
                exclude = set()
                for bf in OUTPUT_DIR.rglob("batch_*.json"):
                    try:
                        with open(bf) as f:
                            for item in json.load(f):
                                ih = item.get("infohash", "")
                                if ih and len(ih) == 40:
                                    exclude.add(bytes.fromhex(ih))
                    except Exception:
                        pass
                cur.execute("DROP TABLE IF EXISTS _exclude")
                cur.execute("CREATE TEMP TABLE _exclude (infohash bytea PRIMARY KEY)")
                ih_list = list(exclude)
                for i in range(0, len(ih_list), 5000):
                    chunk = ih_list[i:i+5000]
                    vals = ",".join(["(%s)"] * len(chunk))
                    cur.execute(f"INSERT INTO _exclude VALUES {vals}", chunk)
                conn.commit()
        except Exception as e:
            print(f"  [reconnect] {e}", flush=True)
            conn = get_connection()
            cur = setup_exclude_table(conn, set())
            exclude = set()
            for bf in OUTPUT_DIR.rglob("batch_*.json"):
                try:
                    with open(bf) as f:
                        for item in json.load(f):
                            ih = item.get("infohash", "")
                            if ih and len(ih) == 40:
                                exclude.add(bytes.fromhex(ih))
                except Exception:
                    pass
            cur.execute("DROP TABLE IF EXISTS _exclude")
            cur.execute("CREATE TEMP TABLE _exclude (infohash bytea PRIMARY KEY)")
            ih_list = list(exclude)
            for i in range(0, len(ih_list), 5000):
                chunk = ih_list[i:i+5000]
                vals = ",".join(["(%s)"] * len(chunk))
                cur.execute(f"INSERT INTO _exclude VALUES {vals}", chunk)
            conn.commit()

        query = query_template.format(limit=BATCH_SIZE)
        try:
            cur.execute(query)
            rows = cur.fetchall()
        except Exception as e:
            print(f"  [error] {e}", flush=True)
            conn = get_connection()
            cur = setup_exclude_table(conn, set())
            continue

        if not rows:
            print(f"  [empty] {category} batch {batch_num}: no more results", flush=True)
            break

        batch = []
        for row in rows:
            ih_hex, name, fc, ts, raw_dirs = row
            dirs = flatten_files(raw_dirs) if raw_dirs else []
            batch.append({
                "infohash": ih_hex,
                "name": name or "",
                "file_count": fc or 0,
                "total_size_bytes": ts or 0,
                "top_dirs": dirs[:10],
            })
            try:
                cur.execute("INSERT INTO _exclude VALUES (%s) ON CONFLICT DO NOTHING", (bytes.fromhex(ih_hex),))
            except ValueError:
                pass

        conn.commit()

        with open(batch_file, "w", encoding="utf-8") as f:
            json.dump(batch, f, ensure_ascii=False, indent=2)

        total_extracted += len(batch)
        print(f"  [ok] {batch_file.name}: {len(batch)} items", flush=True)

    if conn and not conn.closed:
        conn.close()
    return total_extracted


def main():
    print("Loading existing infohashes to exclude...", flush=True)
    exclude_ihs = load_existing_infohashes()
    print(f"Loaded {len(exclude_ihs)} existing infohashes\n", flush=True)

    print("=" * 60, flush=True)
    for sub_query_name, query_template in SUB_QUERIES.items():
        n_batches = BATCHES_PER_QUERY.get(sub_query_name, 10)
        output_cat = CATEGORY_MAP[sub_query_name]
        print(f"\n[{sub_query_name}] -> {output_cat} ({n_batches} batches x {BATCH_SIZE} items)", flush=True)
        try:
            count = extract_batches(sub_query_name, query_template, n_batches)
            print(f"  Total extracted: {count}", flush=True)
        except Exception as e:
            print(f"  ERROR: {e}", flush=True)

    print("\n" + "=" * 60, flush=True)
    print("Done! Check labeling/batches/ for output files.\n", flush=True)
    print("Summary:", flush=True)
    for cat_dir in sorted(OUTPUT_DIR.iterdir()):
        if cat_dir.is_dir():
            batches = list(cat_dir.glob("batch_*.json"))
            total = 0
            for b in batches:
                try:
                    with open(b) as f:
                        total += len(json.load(f))
                except Exception:
                    pass
            print(f"  {cat_dir.name:<15}: {len(batches):>3} batches, {total:>5} items", flush=True)


if __name__ == "__main__":
    main()
