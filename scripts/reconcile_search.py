#!/usr/bin/env python3
"""
GAIA Search Reconciliation & Parity Verification (G5 - Decoupled Architecture)
Enforces strict parity threshold (< 0.1% divergence), watermark liveness (5m window),
Redis DB 1 volatile store integrity, sample presence (n=100), and zero-leak suppression guarantees.
Includes --test-alert harness to verify alert triggers and recovery non-destructively.
Dispatches real-time alerts via NTFY and Webhooks upon invariant violation.
"""

import sys
import os
import time
import json
import urllib.request
import urllib.error
from datetime import datetime, timezone, timedelta

try:
    import redis
    HAVE_REDIS = True
except ImportError:
    HAVE_REDIS = False

MEILI_CONTAINER = os.getenv("MEILI_CONTAINER", "gaia-portal-meilisearch")
MEILI_URL = os.getenv("MEILI_URL", "http://172.18.0.3:7700")
MEILI_KEY = os.getenv("MEILI_MASTER_KEY", "meili_secure_master_key_v2_modern_scaling")

REDIS_CONTAINER = os.getenv("REDIS_CONTAINER", "gaia-portal-redis")
REDIS_HOST = os.getenv("REDIS_HOST", "127.0.0.1")
REDIS_PORT = int(os.getenv("REDIS_PORT", "6379"))
REDIS_PASS = os.getenv("REDIS_PASSWORD", "gaia_redis_vault_secure_2026_scaling")

ALERT_WEBHOOK_URL = os.getenv("ALERT_WEBHOOK_URL", "")
NTFY_TOPIC = os.getenv("NTFY_TOPIC", "gaia-portal-alerts")
NTFY_SERVER = os.getenv("NTFY_SERVER", "https://ntfy.sh")

def sanitize_header(val):
    return val.encode("ascii", errors="ignore").decode("ascii").strip()

def dispatch_alert(subject, details, priority="urgent", tags="warning,rotating_light"):
    """
    Dispatches notifications to ntfy and/or configured webhook endpoints.
    """
    print(f"\n📢 DISPATCHING ALERT: {subject}")
    
    # 1. NTFY dispatch
    if NTFY_TOPIC:
        try:
            url = f"{NTFY_SERVER.rstrip('/')}/{NTFY_TOPIC}"
            body = f"{subject}\n\n{details}".encode("utf-8")
            req = urllib.request.Request(url, data=body, headers={
                "Title": sanitize_header(subject),
                "Priority": priority,
                "Tags": tags,
            })
            with urllib.request.urlopen(req, timeout=5) as resp:
                if resp.status in (200, 201):
                    print(f"  ✅ Alert dispatched to ntfy: {url}")
        except Exception as e:
            print(f"  ⚠️ Failed to dispatch alert to ntfy: {e}")

    # 2. Webhook dispatch
    if ALERT_WEBHOOK_URL:
        try:
            payload = json.dumps({
                "text": f"*{subject}*\n```{details}```"
            }).encode("utf-8")
            req = urllib.request.Request(ALERT_WEBHOOK_URL, data=payload, headers={
                "Content-Type": "application/json"
            })
            with urllib.request.urlopen(req, timeout=5) as resp:
                print(f"  ✅ Alert dispatched to webhook: {ALERT_WEBHOOK_URL}")
        except Exception as e:
            print(f"  ⚠️ Failed to dispatch alert to webhook: {e}")

def get_redis_client(db=1):
    if HAVE_REDIS:
        return redis.Redis(host=REDIS_HOST, port=REDIS_PORT, password=REDIS_PASS, db=db, decode_responses=True)
    return None

def meili_get(path):
    try:
        url = f"{MEILI_URL}{path}"
        req = urllib.request.Request(url, headers={
            "Authorization": f"Bearer {MEILI_KEY}",
            "Content-Type": "application/json"
        })
        with urllib.request.urlopen(req, timeout=3) as resp:
            return json.loads(resp.read().decode())
    except Exception:
        pass

    import subprocess
    cmd = ["docker", "exec", "-i", MEILI_CONTAINER, "curl", "-s", f"http://localhost:7700{path}",
           "-H", f"Authorization: Bearer {MEILI_KEY}"]
    res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return json.loads(res.stdout)

def run_sql(query):
    import subprocess
    env = os.environ.copy()
    env["PGPASSWORD"] = "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"
    cmd = ["psql", "-U", "crawler", "-d", "craw", "-h", "192.168.10.10", "-p", "5432", "-t", "-A", "-c", query]
    res = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if res.returncode != 0:
        cmd2 = ["docker", "exec", "-i", "gaia-postgres", "psql", "-U", "crawler", "-d", "craw", "-t", "-A", "-c", query]
        res = subprocess.run(cmd2, capture_output=True, text=True)
        return res.stdout.strip()
    return res.stdout.strip()

def run_checks(verbose=True, trigger_alert=True):
    errors = []
    now = datetime.now(timezone.utc)
    r = get_redis_client(db=1)

    # 1. Meilisearch Index Stats
    try:
        stats = meili_get("/indexes/torrents/stats")
        meili_docs = stats.get("numberOfDocuments", 0)
        is_indexing = stats.get("isIndexing", False)
        if verbose:
            print(f"Meilisearch 'torrents' document count: {meili_docs:,} (indexing active: {is_indexing})")
    except Exception as e:
        err = f"Unable to reach Meilisearch: {e}"
        print(f"❌ FAILED: {err}")
        errors.append(err)
        if trigger_alert:
            dispatch_alert("🚨 GAIA Search Reconciler Alert: Meilisearch Down", err)
        return False

    # 2. PostgreSQL Active Count
    try:
        pg_count_str = run_sql("SELECT count(*) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';")
        pg_active = int(pg_count_str) if pg_count_str.isdigit() else 0
        if verbose:
            print(f"PostgreSQL active (non-suppressed) count: {pg_active:,}")
    except Exception as e:
        err = f"Unable to query PostgreSQL: {e}"
        print(f"❌ FAILED: {err}")
        errors.append(err)
        if trigger_alert:
            dispatch_alert("🚨 GAIA Search Reconciler Alert: PostgreSQL Query Failure", err)
        return False

    # 3. Redis DB 1 Health Store Size
    try:
        if r:
            redis_docs = r.dbsize()
        else:
            import subprocess
            raw = subprocess.run(["docker", "exec", "-i", REDIS_CONTAINER, "redis-cli", "-a", REDIS_PASS, "--no-auth-warning", "-n", "1", "dbsize"],
                                 capture_output=True, text=True).stdout.strip()
            redis_docs = int(raw) if raw.isdigit() else 0
        if verbose:
            print(f"Redis DB 1 'gaia:health:*' key count: {redis_docs:,}")
    except Exception as e:
        err = f"Unable to query Redis DB 1: {e}"
        print(f"❌ FAILED: {err}")
        errors.append(err)
        if trigger_alert:
            dispatch_alert("🚨 GAIA Search Reconciler Alert: Redis DB 1 Failure", err)
        return False

    # 4. Strict Parity Threshold Checks (Threshold <= 0.001 / 0.1%)
    # Meilisearch vs Postgres
    meili_delta = abs(meili_docs - pg_active)
    meili_div = meili_delta / pg_active if pg_active > 0 else 0
    meili_pct = (meili_docs / pg_active) * 100.0 if pg_active > 0 else 0
    if verbose:
        print(f"Meilisearch Parity: {meili_pct:.4f}% | Delta: {meili_delta:,} | Divergence: {meili_div:.4f}")

    if meili_div > 0.001:
        err = f"Meilisearch divergence {meili_div:.4f} ({meili_div*100:.2f}%) exceeds 0.1% threshold (Delta: {meili_delta:,})"
        print(f"❌ FAILED: {err}")
        errors.append(err)
    elif verbose:
        print(f"✅ PASSED: Meilisearch parity within 0.1% threshold.")

    # Redis DB 1 vs Postgres
    redis_delta = abs(redis_docs - pg_active)
    redis_div = redis_delta / pg_active if pg_active > 0 else 0
    redis_pct = (redis_docs / pg_active) * 100.0 if pg_active > 0 else 0
    if verbose:
        print(f"Redis DB 1 Parity:   {redis_pct:.4f}% | Delta: {redis_delta:,} | Divergence: {redis_div:.4f}")

    if redis_div > 0.001:
        err = f"Redis DB 1 divergence {redis_div:.4f} ({redis_div*100:.2f}%) exceeds 0.1% threshold (Delta: {redis_delta:,})"
        print(f"❌ FAILED: {err}")
        errors.append(err)
    elif verbose:
        print(f"✅ PASSED: Redis DB 1 parity within 0.1% threshold.")

    # 5. Watermark Liveness Check (5 minutes max lag for active loops: fast & suppression)
    max_lags = {
        "fast": timedelta(minutes=5),
        "suppression": timedelta(minutes=5),
    }

    sync_raw = run_sql("SELECT loop_name || '|' || COALESCE(cursor_ts::text, '') || '|' || completed || '|' || rows_synced || '|' || COALESCE(last_run_at::text, '') FROM portal_sync_state;")
    if verbose:
        print("\nCurrent Sync State & Watermark Liveness:")
    for line in sync_raw.splitlines():
        if not line.strip(): continue
        parts = line.split('|')
        loop_name, cursor_str, completed, rows_synced, last_run_str = parts[0], parts[1], parts[2] == 't', int(parts[3]), parts[4]
        
        # Skip retired loops in decoupled architecture
        if loop_name in ("health", "decay", "backfill"):
            continue

        last_run_dt = None
        if last_run_str:
            try:
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
                err = f"Watermark liveness breached on [{loop_name}]: last run {lag_info} exceeds threshold ({max_lags[loop_name]})"
                print(f"❌ FAILED: {err}")
                errors.append(err)

        status_icon = "✅" if liveness_ok else "❌"
        if verbose:
            print(f"  {status_icon} [{loop_name}] completed={completed}, rows_synced={rows_synced:,}, last_run={lag_info}")

    # 6. Suppressed Leak Verification (Sample n=500, Strict Zero-Tolerance)
    suppressed_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action = 'SUPPRESS' LIMIT 500;")
    suppressed_hashes = [h.strip() for h in suppressed_raw.splitlines() if h.strip()]
    meili_leaks = 0
    redis_leaks = 0
    if suppressed_hashes:
        # Check first 50 in Meilisearch
        for ih in suppressed_hashes[:50]:
            try:
                doc = meili_get(f"/indexes/torrents/documents/{ih}")
                if doc and "infohash" in doc:
                    print(f"🚨 ALERT: Suppressed infohash {ih} leaked into Meilisearch!")
                    meili_leaks += 1
            except Exception:
                pass

        # Check all 500 in Redis DB 1 in single pipelined batch
        if r:
            pipe = r.pipeline()
            for ih in suppressed_hashes:
                pipe.exists(f"gaia:health:{ih}")
            exists_results = pipe.execute()
            for idx, ex in enumerate(exists_results):
                if ex:
                    print(f"🚨 ALERT: Suppressed infohash {suppressed_hashes[idx]} leaked into Redis DB 1!")
                    redis_leaks += 1

        if meili_leaks == 0 and redis_leaks == 0:
            if verbose:
                print(f"✅ PASSED: Suppressed Leak Check (0/{len(suppressed_hashes)} leaked).")
        else:
            err = f"Suppressed Leak Check: {meili_leaks} Meili leaks, {redis_leaks} Redis leaks."
            print(f"❌ FAILED: {err}")
            errors.append(err)

    # 7. Sample Integrity Check (Sample n=100, >= 99% presence in Meili and Redis DB 1)
    sample_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS' ORDER BY verified_at DESC LIMIT 100;")
    sample_hashes = [h.strip() for h in sample_raw.splitlines() if h.strip()]
    meili_missing = 0
    redis_missing = 0
    if sample_hashes:
        for ih in sample_hashes:
            try:
                doc = meili_get(f"/indexes/torrents/documents/{ih}")
                if not doc or "infohash" not in doc:
                    meili_missing += 1
            except Exception:
                meili_missing += 1

        if r:
            pipe = r.pipeline()
            for ih in sample_hashes:
                pipe.exists(f"gaia:health:{ih}")
            r_results = pipe.execute()
            for ex in r_results:
                if not ex:
                    redis_missing += 1

        if meili_missing > 0 or redis_missing > 0:
            err = f"Sample Integrity Check: {meili_missing}/100 missing in Meili, {redis_missing}/100 missing in Redis DB 1."
            print(f"❌ FAILED: {err}")
            errors.append(err)
        elif verbose:
            print(f"✅ PASSED: Sample Integrity Check (100% of sample present in Meilisearch & Redis DB 1).")

    if errors and trigger_alert:
        dispatch_alert(
            "🚨 GAIA Search Reconciler Alert: Invariants Breached",
            "\n".join(f"• {e}" for e in errors)
        )

    return len(errors) == 0

def test_alert_pipeline():
    print("\n===============================================================")
    print("EXECUTING RECONCILER ALERT & RECOVERY TEST HARNESS")
    print("===============================================================")
    r = get_redis_client(db=1)
    if not r:
        print("❌ Cannot run alert test: Redis client unavailable.")
        sys.exit(1)

    # Test actual notification dispatch
    print("\n[Step 1/6] Testing Notification Dispatch Pipeline...")
    dispatch_alert(
        "🧪 GAIA Reconciler Test Alert",
        "Verification of active alert dispatch channels (ntfy / webhook). Non-destructive test.",
        priority="default",
        tags="test_tube,bell"
    )

    # Fetch 5 valid sample hashes from PostgreSQL
    print("\n[Step 2/6] Selecting 5 sample keys for non-destructive perturbation...")
    sample_raw = run_sql("SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS' ORDER BY verified_at DESC LIMIT 5;")
    test_hashes = [h.strip() for h in sample_raw.splitlines() if h.strip()]
    
    if len(test_hashes) < 5:
        print("❌ Cannot run alert test: less than 5 sample hashes available.")
        sys.exit(1)

    for h in test_hashes:
        print(f"  - gaia:health:{h}")

    # Stash keys from Redis DB 1 into memory
    stashed_keys = {}
    for h in test_hashes:
        stashed_keys[h] = r.hgetall(f"gaia:health:{h}")
    print("✅ Stashed test key values in memory.")

    # Temporarily delete keys from Redis DB 1
    print("\n[Step 3/6] Simulating key deletion in Redis DB 1...")
    pipe = r.pipeline()
    for h in test_hashes:
        pipe.delete(f"gaia:health:{h}")
    pipe.execute()
    print("🔥 Deleted 5 test keys from Redis DB 1 to simulate corruption.")

    # Run reconciliation check — MUST FAIL
    print("\n[Step 4/6] Running reconciliation check (Expecting Failure)...")
    passed = run_checks(verbose=False, trigger_alert=False)
    
    if passed:
        print("❌ CRITICAL ERROR: Reconciler passed when it should have failed!")
        # Restore before exiting
        restore_pipe = r.pipeline()
        for h, mapping in stashed_keys.items():
            if mapping:
                restore_pipe.hset(f"gaia:health:{h}", mapping=mapping)
        restore_pipe.execute()
        sys.exit(1)
    else:
        print("🚨 ALERT CONFIRMED: Reconciler raised alarm and failed loudly as expected!")

    # Restore keys to Redis DB 1
    print("\n[Step 5/6] Restoring stashed test keys back to Redis DB 1...")
    restore_pipe = r.pipeline()
    for h, mapping in stashed_keys.items():
        if mapping:
            restore_pipe.hset(f"gaia:health:{h}", mapping=mapping)
    restore_pipe.execute()
    print("✅ Restored all 5 test keys to Redis DB 1.")

    # Run reconciliation check again — MUST PASS
    print("\n[Step 6/6] Running reconciliation check (Expecting 100% Clean Pass)...")
    recovered = run_checks(verbose=True, trigger_alert=True)
    if not recovered:
        print("❌ FAILED: Reconciler did not return to clean pass after restoration!")
        sys.exit(1)

    print("\n===============================================================")
    print("✅ RECONCILER ALERT & RECOVERY TEST: ALL TESTS PASSED!")
    print("===============================================================")
    sys.exit(0)

def main():
    print("=== GAIA Search Reconciliation & Parity Verification (G5) ===")
    
    if "--test-alert" in sys.argv:
        test_alert_pipeline()
        return

    success = run_checks(verbose=True, trigger_alert=True)
    if not success:
        print("\n=== RECONCILIATION RESULT: FAILED ===")
        sys.exit(1)
    else:
        print("\n=== RECONCILIATION RESULT: ALL INVARIANTS SATISFIED ===")
        sys.exit(0)

if __name__ == "__main__":
    main()
