"""
Sprint 3: Gold Benchmark Sampler Script.
Extracts 3,000 stratified samples (Set A: 1,800, Set B: 900, Set C: 300) from Postgres.
"""
import os
import sys
import json
from pathlib import Path

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.data.db import get_db_cursor
from src.labels.silver_rules import evaluate_silver_invariants
from src.labels.gold_manager import GoldBenchmarkManager, GOLD_DATA_PATH


def extract_gold_set_a(cur, target_count=1800):
    """Set A: Population-representative stratified sample."""
    print(f"Extracting Gold Set A ({target_count} population-representative samples)...")
    cur.execute("""
        SELECT 
            infohash, name, piece_length, total_size, file_count, 
            files, category, verified_at, first_seen, last_seen,
            seed_confirmed, swarm_peers
        FROM torrents
        TABLESAMPLE SYSTEM (5)
        WHERE total_size > 1024
        LIMIT %s;
    """, (target_count,))
    rows = cur.fetchall()
    samples = []
    for r in rows:
        d = dict(r)
        d["infohash"] = d["infohash"].hex()
        d["set_type"] = "SET_A_POPULATION"
        samples.append(d)
    print(f"  Extracted {len(samples)} Set A samples.")
    return samples


def extract_gold_set_b(cur, target_count=900):
    """Set B: Challenge & uncertainty oversample."""
    print(f"Extracting Gold Set B ({target_count} challenge & anomaly samples)...")
    # Sub-sample 1: small payloads and atypical sizes
    cur.execute("""
        SELECT 
            infohash, name, piece_length, total_size, file_count, 
            files, category, verified_at, first_seen, last_seen,
            seed_confirmed, swarm_peers
        FROM torrents
        WHERE total_size <= 1048576 AND total_size > 1024
        ORDER BY verified_at DESC
        LIMIT %s;
    """, (target_count // 2,))
    small_rows = cur.fetchall()

    # Sub-sample 2: unusual file distributions or keyword matches
    cur.execute("""
        SELECT 
            infohash, name, piece_length, total_size, file_count, 
            files, category, verified_at, first_seen, last_seen,
            seed_confirmed, swarm_peers
        FROM torrents
        WHERE name ~* '(crack|keygen|patch|free download|repack)'
        ORDER BY verified_at DESC
        LIMIT %s;
    """, (target_count - len(small_rows),))
    keyword_rows = cur.fetchall()

    samples = []
    for r in list(small_rows) + list(keyword_rows):
        d = dict(r)
        d["infohash"] = d["infohash"].hex()
        d["set_type"] = "SET_B_CHALLENGE"
        samples.append(d)
    print(f"  Extracted {len(samples)} Set B samples.")
    return samples


def extract_gold_set_c(cur, target_count=300):
    """Set C: Critical Invariants (150 known malicious + 150 known clean)."""
    print(f"Extracting Gold Set C ({target_count} critical invariants)...")
    # 150 malicious (empty payloads, deceptive extensions, sha1 errors)
    cur.execute("""
        SELECT 
            infohash, name, piece_length, total_size, file_count, 
            files, category, verified_at, first_seen, last_seen,
            seed_confirmed, swarm_peers
        FROM torrents
        WHERE total_size <= 1024 OR name ~* '\\.(mp4|avi|mkv)\\.exe$'
        LIMIT 150;
    """)
    malicious_rows = cur.fetchall()

    # 150 clean canonical releases (multi-GB confirmed seeds)
    cur.execute(r"""
        SELECT 
            infohash, name, piece_length, total_size, file_count, 
            files, category, verified_at, first_seen, last_seen,
            seed_confirmed, swarm_peers
        FROM torrents
        WHERE seed_confirmed = true AND total_size > 524288000
          AND (name ~* '\.1080p\.' OR name ~* '\.720p\.' OR name ~* 'bluray' OR name ~* 'web-dl')
        LIMIT 150;
    """)
    clean_rows = cur.fetchall()

    samples = []
    for r in malicious_rows:
        d = dict(r)
        d["infohash"] = d["infohash"].hex()
        d["set_type"] = "SET_C_CRITICAL_MALICIOUS"
        d["gold_label"] = "SUPPRESS"
        samples.append(d)

    for r in clean_rows:
        d = dict(r)
        d["infohash"] = d["infohash"].hex()
        d["set_type"] = "SET_C_CRITICAL_CLEAN"
        d["gold_label"] = "ALLOW"
        samples.append(d)

    print(f"  Extracted {len(samples)} Set C samples ({len(malicious_rows)} malicious, {len(clean_rows)} clean).")
    return samples


def build_and_save_gold_benchmark():
    with get_db_cursor() as cur:
        set_a = extract_gold_set_a(cur, 1800)
        set_b = extract_gold_set_b(cur, 900)
        set_c = extract_gold_set_c(cur, 300)

    all_samples = set_a + set_b + set_c
    print(f"\nTotal Gold Benchmark Samples: {len(all_samples):,}")

    mgr = GoldBenchmarkManager()
    mgr.save_benchmark(all_samples)
    print(f"Successfully saved gold benchmark to {GOLD_DATA_PATH}")


if __name__ == "__main__":
    build_and_save_gold_benchmark()
