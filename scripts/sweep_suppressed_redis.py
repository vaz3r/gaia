#!/usr/bin/env python3
"""
One-off Historical Sweep: Purge all suppressed torrents from Redis DB 1.
Fetches all policy_action = 'SUPPRESS' infohashes from PostgreSQL
and pipelines DEL commands into Redis DB 1.
"""
import os
import sys
import subprocess
import redis

REDIS_HOST = os.getenv("REDIS_HOST", "127.0.0.1")
REDIS_PORT = int(os.getenv("REDIS_PORT", "6379"))
REDIS_PASS = os.getenv("REDIS_PASSWORD", "gaia_redis_vault_secure_2026_scaling")
REDIS_DB = 1

def main():
    print(f"Connecting to Redis {REDIS_HOST}:{REDIS_PORT} DB {REDIS_DB}...")
    r = redis.Redis(host=REDIS_HOST, port=REDIS_PORT, password=REDIS_PASS, db=REDIS_DB, decode_responses=True)
    
    print("Streaming suppressed infohashes from PostgreSQL...")
    env = os.environ.copy()
    env["PGPASSWORD"] = "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"
    cmd = [
        "psql", "-U", "crawler", "-d", "craw", "-h", "192.168.10.10", "-p", "5432",
        "-t", "-A", "-c", "SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action = 'SUPPRESS';"
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True, env=env)

    total_scanned = 0
    total_deleted = 0
    batch = []
    batch_size = 5000

    def process_batch(hashes):
        nonlocal total_deleted
        if not hashes:
            return
        pipe = r.pipeline()
        for h in hashes:
            pipe.delete(f"gaia:health:{h}")
        results = pipe.execute()
        total_deleted += sum(results)

    for line in proc.stdout:
        ih = line.strip()
        if not ih or len(ih) != 40:
            continue
        batch.append(ih)
        total_scanned += 1
        if len(batch) >= batch_size:
            process_batch(batch)
            batch = []
            if total_scanned % 25000 == 0:
                print(f"Scanned {total_scanned:,} hashes... (deleted so far: {total_deleted:,})")

    if batch:
        process_batch(batch)

    proc.wait()
    print("==================================================")
    print(f"Sweep Completed!")
    print(f"Total Suppressed Hashes Scanned: {total_scanned:,}")
    print(f"Total Leaked Redis Keys Deleted:  {total_deleted:,}")
    print(f"Remaining Redis DB 1 Size:        {r.dbsize():,}")
    print("==================================================")

if __name__ == "__main__":
    main()
