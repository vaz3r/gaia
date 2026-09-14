#!/usr/bin/env python3
"""
GAIA PostgreSQL to Quickwit Synchronization Script
Streams verified torrent records from PostgreSQL into the Quickwit 'torrents' index using NDJSON batches.
"""

import os
import sys
import time
import json
import argparse
import psycopg2
import urllib.request
import urllib.error

DB_HOST = os.getenv("DB_HOST", "localhost")
DB_PORT = int(os.getenv("PG_PORT", "5432"))
DB_USER = os.getenv("POSTGRES_USER", "crawler")
DB_PASS = os.getenv("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b")
DB_NAME = os.getenv("POSTGRES_DB", "craw")
QUICKWIT_URL = os.getenv("QUICKWIT_URL", "http://127.0.0.1:7280")
BATCH_SIZE = int(os.getenv("BATCH_SIZE", "5000"))

def get_db_connection():
    return psycopg2.connect(
        host=DB_HOST,
        port=DB_PORT,
        user=DB_USER,
        password=DB_PASS,
        dbname=DB_NAME
    )

def post_ndjson_batch(ndjson_data: str, max_retries: int = 5) -> bool:
    url = f"{QUICKWIT_URL}/api/v1/torrents/ingest"
    req = urllib.request.Request(
        url,
        data=ndjson_data.encode("utf-8"),
        headers={"Content-Type": "application/x-ndjson"},
        method="POST"
    )
    for attempt in range(max_retries):
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                return resp.status in (200, 202)
        except urllib.error.HTTPError as e:
            body = e.read().decode("utf-8", errors="replace")
            print(f"\n[ERROR] Quickwit returned {e.code}: {body} (attempt {attempt+1}/{max_retries})", file=sys.stderr)
            time.sleep(3)
        except Exception as e:
            print(f"\n[WARN] Request attempt {attempt+1}/{max_retries} failed: {e}", file=sys.stderr)
            time.sleep(3)
    return False

def sync_records(limit: int = 0, offset: int = 0):
    conn = get_db_connection()
    count_cursor = conn.cursor()
    count_cursor.execute("SELECT count(*) FROM torrents WHERE name IS NOT NULL;")
    total_in_db = count_cursor.fetchone()[0]
    count_cursor.close()

    total_target = min(limit, total_in_db - offset) if limit > 0 else (total_in_db - offset)
    print(f"[*] Starting Quickwit Sync: {total_target:,} records target (Total DB: {total_in_db:,}, Offset: {offset:,})")
    print(f"[*] Batch size: {BATCH_SIZE:,} | Target Index: {QUICKWIT_URL}/api/v1/torrents")

    query = """
        SELECT 
            encode(infohash, 'hex') as infohash,
            name,
            COALESCE(category, 'Other') as category,
            COALESCE(total_size, 0) as total_size,
            COALESCE(file_count, 0) as file_count,
            EXTRACT(EPOCH FROM verified_at)::bigint as verified_at,
            COALESCE(health_score, 0) as health_score,
            COALESCE(popularity_score, 0) as popularity_score,
            COALESCE(swarm_peers, 0) as swarm_peers,
            COALESCE(seed_confirmed, false) as seed_confirmed,
            COALESCE(risk_tier, 'SAFE') as risk_tier,
            COALESCE(policy_action, 'ALLOW') as policy_action
        FROM torrents
        WHERE name IS NOT NULL
        ORDER BY verified_at DESC
    """
    if limit > 0:
        query += f" LIMIT {limit}"
    if offset > 0:
        query += f" OFFSET {offset}"

    cursor = conn.cursor(name="quickwit_sync_cursor")
    cursor.itersize = BATCH_SIZE
    cursor.execute(query)

    start_time = time.time()
    total_processed = 0
    batch = []

    try:
        while True:
            rows = cursor.fetchmany(BATCH_SIZE)
            if not rows:
                break

            ndjson_lines = []
            for row in rows:
                doc = {
                    "infohash": row[0],
                    "name": row[1],
                    "category": row[2],
                    "total_size": int(row[3]),
                    "file_count": int(row[4]),
                    "verified_at": int(row[5]) if row[5] else int(time.time()),
                    "health_score": int(row[6]),
                    "popularity_score": int(row[7]),
                    "swarm_peers": int(row[8]),
                    "seed_confirmed": bool(row[9]),
                    "risk_tier": row[10],
                    "policy_action": row[11]
                }
                ndjson_lines.append(json.dumps(doc))

            ndjson_payload = "\n".join(ndjson_lines) + "\n"
            if not post_ndjson_batch(ndjson_payload):
                print(f"[!] Retrying batch at {total_processed:,}...")
                time.sleep(2)
                if not post_ndjson_batch(ndjson_payload):
                    print(f"[FAILED] Could not post batch at offset {total_processed:,}", file=sys.stderr)
                    break

            total_processed += len(rows)
            elapsed = time.time() - start_time
            rate = total_processed / elapsed if elapsed > 0 else 0
            pct = (total_processed / total_target) * 100 if total_target > 0 else 0
            eta_sec = (total_target - total_processed) / rate if rate > 0 else 0
            eta_min = eta_sec / 60

            sys.stdout.write(
                f"\r[Syncing] {total_processed:,} / {total_target:,} ({pct:.1f}%) "
                f"| {rate:,.0f} rec/s | ETA: {eta_min:.1f}m"
            )
            sys.stdout.flush()

        print(f"\n[DONE] Successfully indexed {total_processed:,} records in {time.time() - start_time:.1f}s!")

    finally:
        cursor.close()
        conn.close()

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Sync PostgreSQL records to Quickwit")
    parser.add_argument("--limit", type=int, default=0, help="Maximum number of records to sync (0 = all)")
    parser.add_argument("--offset", type=int, default=0, help="Offset to resume from")
    args = parser.parse_args()
    sync_records(args.limit, args.offset)
