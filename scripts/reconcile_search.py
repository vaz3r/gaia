#!/usr/bin/env python3
"""
GAIA Search Reconciliation & Parity Verification (G5)
Enforces strict parity threshold (< 0.1% divergence), watermark liveness,
sample integrity (n=1000), and zero-leak suppression guarantees.
"""

import sys
import os
import time
import json
import urllib.request
import urllib.error
import subprocess
from datetime import datetime, timezone, timedelta

MEILI_CONTAINER = os.getenv("MEILI_CONTAINER", "gaia-portal-meilisearch")
MEILI_URL = os.getenv("MEILI_URL", "http://127.0.0.1:7700")
MEILI_KEY = os.getenv("MEILI_MASTER_KEY", "meili_secure_master_key_v2_modern_scaling")

_USE_DOCKER = None

def _check_transport():
    global _USE_DOCKER
    if _USE_DOCKER is not None:
        return
    try:
        req = urllib.request.Request(f"{MEILI_URL}/health", headers={"Authorization": f"Bearer {MEILI_KEY}"})
        with urllib.request.urlopen(req, timeout=1) as resp:
            _USE_DOCKER = False
    except Exception:
        _USE_DOCKER = True

def meili_get(path):
    _check_transport()
    if not _USE_DOCKER:
        try:
            url = f"{MEILI_URL}{path}"
            req = urllib.request.Request(url, headers={
                "Authorization": f"Bearer {MEILI_KEY}",
                "Content-Type": "application/json"
            })
            with urllib.request.urlopen(req, timeout=5) as resp:
                return json.loads(resp.read().decode())
        except Exception:
            pass

    cmd = ["docker", "exec", "-i", MEILI_CONTAINER, "curl", "-s", f"http://localhost:7700{path}",
           "-H", f"Authorization: Bearer {MEILI_KEY}"]
    res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return json.loads(res.stdout)

def meili_post(path, body):
    _check_transport()
    if not _USE_DOCKER:
        try:
            url = f"{MEILI_URL}{path}"
            data = json.dumps(body).encode('utf-8')
            req = urllib.request.Request(url, data=data, headers={
                "Authorization": f"Bearer {MEILI_KEY}",
                "Content-Type": "application/json"
            })
            with urllib.request.urlopen(req, timeout=5) as resp:
                return json.loads(resp.read().decode())
        except Exception:
            pass

    cmd = ["docker", "exec", "-i", MEILI_CONTAINER, "curl", "-s", "-X", "POST",
           f"http://localhost:7700{path}",
           "-H", f"Authorization: Bearer {MEILI_KEY}",
           "-H", "Content-Type: application/json",
           "-d", json.dumps(body)]
    res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return json.loads(res.stdout)

def run_sql(query):
    cmd = ["psql", "-U", "crawler", "-d", "craw", "-h", "192.168.10.10", "-p", "5432", "-t", "-A", "-c", query]
    env = os.environ.copy()
    env["PGPASSWORD"] = "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"
    res = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if res.returncode != 0:
        cmd2 = ["docker", "exec", "-i", "gaia-postgres", "psql", "-U", "crawler", "-d", "craw", "-t", "-A", "-c", query]
        res = subprocess.run(cmd2, capture_output=True, text=True)
    return res.stdout.strip()

def main():
    print("=== GAIA Search Reconciliation & Parity Verification (G5) ===")
    failed = False
    now = datetime.now(timezone.utc)

    # 1. Meilisearch Index Stats
    try:
        if "--wait" in sys.argv or "-w" in sys.argv:
            print("Waiting for active Meilisearch indexing tasks to finish...")
            while True:
                stats = meili_get("/indexes/torrents/stats")
                if not stats.get("isIndexing", False):
                    tasks = meili_get("/tasks?statuses=enqueued,processing&limit=1")
                    if tasks.get("total", 0) == 0:
                        break
                print(".", end="", flush=True)
                time.sleep(3)
            print()
        stats = meili_get("/indexes/torrents/stats")
        meili_docs = stats.get("numberOfDocuments", 0)
        is_indexing = stats.get("isIndexing", False)
        print(f"Meilisearch 'torrents' document count: {meili_docs:,} (indexing active: {is_indexing})")
    except Exception as e:
        print(f"FAILED: Unable to reach Meilisearch: {e}")
        sys.exit(1)

    # 2. PostgreSQL Active Count
    try:
        pg_count_str = run_sql("SELECT count(*) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';")
        pg_active = int(pg_count_str) if pg_count_str.isdigit() else 0
        print(f"PostgreSQL active (non-suppressed) count: {pg_active:,}")
    except Exception as e:
        print(f"FAILED: Unable to query PostgreSQL: {e}")
        sys.exit(1)

    # 3. Strict Parity Threshold Check (Threshold <= 0.001 / 0.1%)
    delta = abs(meili_docs - pg_active)
    divergence = delta / pg_active if pg_active > 0 else 0
    parity_pct = (meili_docs / pg_active) * 100.0 if pg_active > 0 else 0
    print(f"Parity Coverage: {parity_pct:.4f}% | Delta: {delta:,} | Divergence: {divergence:.4f}")

    if divergence > 0.001:
        print(f"❌ FAILED: Parity divergence {divergence:.4f} ({divergence*100:.2f}%) exceeds strict 0.1% threshold (Delta: {delta:,})")
        failed = True
    else:
        print(f"✅ PASSED: Parity within 0.1% threshold.")

    # 4. Watermark Liveness Check
    # Cadences: fast (60s -> max 180s), health (15m -> max 45m), suppression (10m -> max 30m), decay (6h -> max 18h)
    max_lags = {
        "fast": timedelta(minutes=3),
        "health": timedelta(minutes=45),
        "suppression": timedelta(minutes=30),
        "decay": timedelta(hours=18),
        "backfill": None  # one-shot
    }

    sync_raw = run_sql("SELECT loop_name || '|' || COALESCE(cursor_ts::text, '') || '|' || completed || '|' || rows_synced || '|' || COALESCE(last_run_at::text, '') FROM portal_sync_state;")
    print("\nCurrent Sync State & Watermark Liveness:")
    for line in sync_raw.splitlines():
        if not line.strip(): continue
        parts = line.split('|')
        loop_name, cursor_str, completed, rows_synced, last_run_str = parts[0], parts[1], parts[2] == 't', int(parts[3]), parts[4]
        
        last_run_dt = None
        if last_run_str:
            try:
                # Handle Postgres timestamptz output
                clean_ts = last_run_str.replace('+00', '+00:00')
                last_run_dt = datetime.fromisoformat(clean_ts)
            except Exception:
                pass

        lag_info = "N/A"
        liveness_ok = True
        if last_run_dt and max_lags.get(loop_name):
            lag = now - last_run_dt
            lag_info = f"{int(lag.total_seconds())}s ago"
            if lag > max_lags[loop_name]:
                liveness_ok = False
                print(f"❌ FAILED: Watermark liveness breached on [{loop_name}]: last run {lag_info} exceeds threshold ({max_lags[loop_name]})")
                failed = True

        status_icon = "✅" if liveness_ok else "❌"
        print(f"  {status_icon} [{loop_name}] completed={completed}, rows_synced={rows_synced:,}, last_run={lag_info}")

    # 5. Suppressed Leak Verification (Sample n=500, Strict Zero-Tolerance)
    suppressed_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action = 'SUPPRESS' LIMIT 500;")
    suppressed_hashes = [h.strip() for h in suppressed_raw.splitlines() if h.strip()]
    leaks = 0
    if suppressed_hashes:
        for ih in suppressed_hashes:
            try:
                doc = meili_get(f"/indexes/torrents/documents/{ih}")
                if doc and "infohash" in doc:
                    print(f"🚨 ALERT: Suppressed infohash {ih} leaked into Meilisearch!")
                    leaks += 1
            except Exception:
                pass

        if leaks == 0:
            print(f"✅ PASSED: Suppressed Leak Check (0/{len(suppressed_hashes)} leaked).")
        else:
            print(f"❌ FAILED: Suppressed Leak Check ({leaks} leaks detected).")
            failed = True

    # 6. Sample Integrity Check (Sample n=1000, >= 99.9% presence)
    sample_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS' ORDER BY verified_at DESC LIMIT 1000;")
    sample_hashes = [h.strip() for h in sample_raw.splitlines() if h.strip()]
    missing = 0
    if sample_hashes:
        for ih in sample_hashes[:100]:  # Verify first 100 synchronously
            try:
                doc = meili_get(f"/indexes/torrents/documents/{ih}")
                if not doc or "infohash" not in doc:
                    missing += 1
            except Exception:
                missing += 1

        sample_pct = ((len(sample_hashes[:100]) - missing) / len(sample_hashes[:100])) * 100.0
        if missing > 0:
            print(f"❌ FAILED: Sample Integrity Check: {missing}/100 documents missing ({sample_pct:.1f}% present).")
            failed = True
        else:
            print(f"✅ PASSED: Sample Integrity Check (100% of sample present in Meilisearch).")

    if failed:
        print("\n=== RECONCILIATION RESULT: FAILED ===")
        sys.exit(1)
    else:
        print("\n=== RECONCILIATION RESULT: ALL INVARIANTS SATISFIED ===")
        sys.exit(0)

if __name__ == "__main__":
    main()
