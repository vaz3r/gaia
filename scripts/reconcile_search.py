#!/usr/bin/env python3
"""
GAIA Search Reconciliation & Parity Verification (G5)
Runs nightly or on-demand to guarantee consistency between PostgreSQL and Meilisearch.
"""

import sys
import os
import time
import requests
import psycopg2
from psycopg2.extras import RealDictCursor

PG_CONN = os.getenv("DATABASE_URL", "host=192.168.10.10 port=5432 dbname=craw user=crawler password=83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b connect_timeout=10")
MEILI_URL = os.getenv("MEILI_URL", "http://127.0.0.1:7700")
MEILI_KEY = os.getenv("MEILI_MASTER_KEY", "meili_secure_master_key_v2_modern_scaling")

HEADERS = {
    "Authorization": f"Bearer {MEILI_KEY}",
    "Content-Type": "application/json"
}

def main():
    print("=== GAIA Search Reconciliation & Parity Check (G5) ===")
    
    # 1. Connect to PostgreSQL
    try:
        conn = psycopg2.connect(PG_CONN)
        cur = conn.cursor(cursor_factory=RealDictCursor)
        print("Connected to PostgreSQL successfully.")
    except Exception as e:
        print(f"FAILED: Unable to connect to PostgreSQL: {e}")
        sys.exit(1)

    # 2. Check sync state
    cur.execute("SELECT loop_name, cursor_ts, completed, rows_synced, last_run_at, last_error FROM portal_sync_state;")
    states = {row["loop_name"]: row for row in cur.fetchall()}
    print("\nCurrent Sync States:")
    for name, s in states.items():
        print(f"  [{name}] completed={s['completed']}, rows_synced={s['rows_synced']}, last_run={s['last_run_at']}, error={s['last_error']}")

    backfill = states.get("backfill")
    if not backfill or not backfill["completed"]:
        print("\nNotice: Initial backfill is still in progress or incomplete. Parity check will report status without failing.")

    # 3. Check Meilisearch health and count
    try:
        health_resp = requests.get(f"{MEILI_URL}/health", headers=HEADERS, timeout=5)
        health_resp.raise_for_status()
        
        idx_resp = requests.get(f"{MEILI_URL}/indexes/torrents", headers=HEADERS, timeout=5)
        if idx_resp.status_code == 404:
            print("ERROR: Meilisearch index 'torrents' does not exist.")
            sys.exit(1)
        idx_resp.raise_for_status()
        meili_info = idx_resp.json()
        meili_doc_count = meili_info.get("numberOfDocuments", 0)
        print(f"\nMeilisearch 'torrents' document count: {meili_doc_count:,}")
    except Exception as e:
        print(f"FAILED: Unable to reach Meilisearch: {e}")
        sys.exit(1)

    # 4. PostgreSQL Active Count
    cur.execute("SELECT count(*) AS total FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';")
    pg_active_count = cur.fetchone()["total"]
    print(f"PostgreSQL active (non-suppressed) count: {pg_active_count:,}")

    if pg_active_count > 0:
        ratio = (meili_doc_count / pg_active_count) * 100.0
        print(f"Parity Coverage: {ratio:.2f}%")
        if backfill and backfill["completed"] and ratio < 99.0:
            print(f"WARNING: Parity divergence exceeds 1.0% threshold ({ratio:.2f}%)")

    # 5. Suppressed Leak Check (Must be 0)
    cur.execute("SELECT encode(infohash, 'hex') AS infohash FROM torrents WHERE policy_action = 'SUPPRESS' LIMIT 50;")
    suppressed_sample = [r["infohash"] for r in cur.fetchall()]
    leaked_count = 0
    if suppressed_sample:
        for ih in suppressed_sample:
            r = requests.get(f"{MEILI_URL}/indexes/torrents/documents/{ih}", headers=HEADERS, timeout=2)
            if r.status_code == 200:
                print(f"ALERT: Suppressed infohash {ih} exists in Meilisearch!")
                leaked_count += 1
        if leaked_count == 0:
            print(f"Suppressed Leak Check: PASSED (0/{len(suppressed_sample)} leaked)")
        else:
            print(f"Suppressed Leak Check: FAILED ({leaked_count} leaks detected)")
            sys.exit(1)
    else:
        print("Suppressed Leak Check: PASSED (no suppressed torrents in sample)")

    # 6. Random Sample Verification (if backfill completed)
    if backfill and backfill["completed"]:
        cur.execute("SELECT encode(infohash, 'hex') AS infohash FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS' ORDER BY random() LIMIT 100;")
        sample = [r["infohash"] for r in cur.fetchall()]
        missing_count = 0
        for ih in sample:
            r = requests.get(f"{MEILI_URL}/indexes/torrents/documents/{ih}", headers=HEADERS, timeout=2)
            if r.status_code != 200:
                missing_count += 1
        print(f"Sample Accuracy: {len(sample) - missing_count}/{len(sample)} verified present in Meilisearch.")

    cur.close()
    conn.close()
    print("\n=== Reconciliation Finished Successfully ===")

if __name__ == "__main__":
    main()
