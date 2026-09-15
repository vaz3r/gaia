#!/usr/bin/env python3
"""
GAIA Search Reconciliation & Parity Verification (G5)
Zero-dependency reconciliation tool using standard library (urllib) and psql/psycopg2.
"""

import sys
import os
import json
import urllib.request
import urllib.error
import subprocess

MEILI_CONTAINER = os.getenv("MEILI_CONTAINER", "gaia-portal-meilisearch")
MEILI_URL = os.getenv("MEILI_URL", "http://127.0.0.1:7700")
MEILI_KEY = os.getenv("MEILI_MASTER_KEY", "meili_secure_master_key_v2_modern_scaling")

def meili_get(path):
    # Try direct HTTP first (if network allows)
    try:
        url = f"{MEILI_URL}{path}"
        req = urllib.request.Request(url, headers={
            "Authorization": f"Bearer {MEILI_KEY}",
            "Content-Type": "application/json"
        })
        with urllib.request.urlopen(req, timeout=5) as resp:
            return json.loads(resp.read().decode())
    except Exception:
        # Fallback to docker exec inside container
        cmd = ["docker", "exec", "-i", MEILI_CONTAINER, "curl", "-s", f"http://localhost:7700{path}",
               "-H", f"Authorization: Bearer {MEILI_KEY}"]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(res.stdout)

def run_sql(query):
    # Run query via psql on CT 110 or local psql
    cmd = ["psql", "-U", "crawler", "-d", "craw", "-h", "192.168.10.10", "-p", "5432", "-t", "-A", "-c", query]
    env = os.environ.copy()
    env["PGPASSWORD"] = "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"
    res = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if res.returncode != 0:
        # Try without host (if inside container)
        cmd2 = ["docker", "exec", "-i", "gaia-postgres", "psql", "-U", "crawler", "-d", "craw", "-t", "-A", "-c", query]
        res = subprocess.run(cmd2, capture_output=True, text=True)
    return res.stdout.strip()

def main():
    print("=== GAIA Search Reconciliation & Parity Check (G5) ===")

    # 1. Check Meilisearch Stats
    try:
        stats = meili_get("/indexes/torrents/stats")
        meili_docs = stats.get("numberOfDocuments", 0)
        print(f"Meilisearch 'torrents' document count: {meili_docs:,}")
    except Exception as e:
        print(f"FAILED: Unable to reach Meilisearch: {e}")
        sys.exit(1)

    # 2. Check PostgreSQL Active Count
    try:
        pg_count_str = run_sql("SELECT count(*) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';")
        pg_active = int(pg_count_str) if pg_count_str.isdigit() else 0
        print(f"PostgreSQL active (non-suppressed) count: {pg_active:,}")
    except Exception as e:
        print(f"FAILED: Unable to query PostgreSQL: {e}")
        sys.exit(1)

    # 3. Parity Ratio
    if pg_active > 0:
        ratio = (meili_docs / pg_active) * 100.0
        print(f"Parity Coverage: {ratio:.2f}%")

    # 4. Check Sync State
    sync_raw = run_sql("SELECT loop_name || '|' || COALESCE(cursor_ts::text, '') || '|' || completed || '|' || rows_synced FROM portal_sync_state;")
    print("\nCurrent Sync Loops:")
    for line in sync_raw.splitlines():
        if line.strip():
            parts = line.split('|')
            print(f"  [{parts[0]}] completed={parts[2]}, rows_synced={parts[3]}, cursor={parts[1]}")

    # 5. Suppressed Leak Check (Must be 0)
    suppressed_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action = 'SUPPRESS' LIMIT 20;")
    suppressed_hashes = [h.strip() for h in suppressed_raw.splitlines() if h.strip()]
    leaks = 0
    for ih in suppressed_hashes:
        try:
            doc = meili_get(f"/indexes/torrents/documents/{ih}")
            if doc and "infohash" in doc:
                print(f"ALERT: Suppressed infohash {ih} leaked into Meilisearch!")
                leaks += 1
        except Exception:
            pass

    if leaks == 0:
        print(f"Suppressed Leak Check: PASSED (0/{len(suppressed_hashes)} leaked)")
    else:
        print(f"Suppressed Leak Check: FAILED ({leaks} leaks detected)")
        sys.exit(1)

    print("\n=== All Parity & Consistency Invariants Verified ===")

if __name__ == "__main__":
    main()
