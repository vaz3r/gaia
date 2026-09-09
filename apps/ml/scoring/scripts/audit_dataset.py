"""
Sprint 1: Dataset & Failure State Audit Script.
Analyzes retry conversion curves, job durations, sighting velocity, and fake torrent distribution.
"""
import os
import sys
import json
from pathlib import Path
from datetime import datetime

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.data.db import get_db_cursor


def audit_verification_jobs():
    print("==================================================")
    print("1. AUDITING VERIFICATION JOBS & RETRY CONVERSION")
    print("==================================================")
    with get_db_cursor() as cur:
        # Job counts by status
        cur.execute("""
            SELECT status, count(*) as cnt,
                   round(count(*)::numeric / sum(count(*)) over () * 100, 2) as pct
            FROM verification_jobs
            GROUP BY status
            ORDER BY cnt DESC;
        """)
        status_rows = cur.fetchall()
        print("\n--- Status Distribution ---")
        for r in status_rows:
            print(f"  {r['status']:<16} : {r['cnt']:>10,} ({r['pct']}%)")

        # Retry count conversion to success
        cur.execute("""
            SELECT retry_count,
                   count(*) as total_jobs,
                   count(*) FILTER (WHERE status = 'success') as successful_jobs,
                   round(count(*) FILTER (WHERE status = 'success')::numeric / nullif(count(*), 0) * 100, 2) as success_rate_pct
            FROM verification_jobs
            WHERE retry_count <= 5
            GROUP BY retry_count
            ORDER BY retry_count ASC;
        """)
        retry_rows = cur.fetchall()
        print("\n--- Retry Count vs Success Rate ---")
        for r in retry_rows:
            print(f"  Retry {r['retry_count']:<2} : Total={r['total_jobs']:>8,}, Success={r['successful_jobs']:>8,} ({r['success_rate_pct']}%)")

        # Top error reasons
        cur.execute("""
            SELECT last_error, count(*) as cnt
            FROM verification_jobs
            WHERE status != 'success' AND last_error IS NOT NULL AND last_error != ''
            GROUP BY last_error
            ORDER BY cnt DESC
            LIMIT 10;
        """)
        error_rows = cur.fetchall()
        print("\n--- Top Last Errors in Failed/Timeout Jobs ---")
        for r in error_rows:
            print(f"  {r['last_error'][:60]:<60} : {r['cnt']:>8,}")

        # Latency to verification from torrents table
        print("\n--- Latency from first_seen to verified_at on Torrents Catalog ---")
        cur.execute("""
            SELECT 
                round(percentile_cont(0.50) WITHIN GROUP (ORDER BY extract(epoch FROM (verified_at - first_seen)))::numeric, 1) as p50_sec,
                round(percentile_cont(0.90) WITHIN GROUP (ORDER BY extract(epoch FROM (verified_at - first_seen)))::numeric, 1) as p90_sec,
                round(percentile_cont(0.99) WITHIN GROUP (ORDER BY extract(epoch FROM (verified_at - first_seen)))::numeric, 1) as p99_sec
            FROM torrents
            WHERE verified_at >= first_seen AND first_seen IS NOT NULL;
        """)
        dur = cur.fetchone()
        if dur:
            print(f"  Verified latency: p50={dur['p50_sec']}s, p90={dur['p90_sec']}s, p99={dur['p99_sec']}s")


def audit_torrents():
    print("\n==================================================")
    print("2. AUDITING TORRENT CATALOG & STRUCTURAL ANOMALIES")
    print("==================================================")
    with get_db_cursor() as cur:
        cur.execute("SELECT count(*) as total_torrents FROM torrents;")
        total = cur.fetchone()['total_torrents']
        print(f"\nTotal Torrents: {total:,}")

        # Size distribution and zero/tiny payloads
        cur.execute("""
            SELECT 
                count(*) FILTER (WHERE total_size = 0) as zero_size,
                count(*) FILTER (WHERE total_size > 0 AND total_size <= 1024) as tiny_size_le_1kb,
                count(*) FILTER (WHERE total_size > 1024 AND total_size <= 1048576) as size_1kb_to_1mb,
                count(*) FILTER (WHERE total_size > 1048576 AND total_size <= 1073741824) as size_1mb_to_1gb,
                count(*) FILTER (WHERE total_size > 1073741824) as size_gt_1gb
            FROM torrents;
        """)
        size_stats = cur.fetchone()
        print("\n--- Size Distribution ---")
        print(f"  0 Bytes            : {size_stats['zero_size']:>8,}")
        print(f"  1 B to 1 KB (Tiny) : {size_stats['tiny_size_le_1kb']:>8,} (Suspicious fake candidate)")
        print(f"  1 KB to 1 MB       : {size_stats['size_1kb_to_1mb']:>8,}")
        print(f"  1 MB to 1 GB       : {size_stats['size_1mb_to_1gb']:>8,}")
        print(f"  > 1 GB             : {size_stats['size_gt_1gb']:>8,}")

        # Categories
        cur.execute("""
            SELECT coalesce(category, 'UNKNOWN') as cat, count(*) as cnt
            FROM torrents
            GROUP BY cat
            ORDER BY cnt DESC
            LIMIT 12;
        """)
        cats = cur.fetchall()
        print("\n--- Category Distribution ---")
        for r in cats:
            print(f"  {r['cat']:<20} : {r['cnt']:>8,}")

        # Known fake sample check (e.g. tiny Windows or video fakes)
        cur.execute("""
            SELECT name, total_size, file_count, category
            FROM torrents
            WHERE (total_size <= 1024 OR name ~* '\\.(mp4|avi|mkv)\\.exe$')
            LIMIT 5;
        """)
        fakes = cur.fetchall()
        print("\n--- Sample Anomalous / Fake Torrents in DB ---")
        for r in fakes:
            print(f"  Name: {r['name'][:50]:<50} | Size: {r['total_size']:>6} B | Files: {r['file_count']} | Cat: {r['category']}")


def audit_timeline():
    print("\n==================================================")
    print("3. AUDITING TIMELINE & SIGHTING HORIZONS")
    print("==================================================")
    with get_db_cursor() as cur:
        cur.execute("""
            SELECT min(first_seen) as min_seen, max(last_seen) as max_seen,
                   count(*) as total_sightings
            FROM infohash_sightings;
        """)
        sighting_stats = cur.fetchone()
        print(f"\nInfohash Sightings Range:")
        print(f"  Min Sighting : {sighting_stats['min_seen']}")
        print(f"  Max Sighting : {sighting_stats['max_seen']}")
        print(f"  Total Rows   : {sighting_stats['total_sightings']:,}")

        cur.execute("""
            SELECT min(updated_at) as min_job, max(updated_at) as max_job
            FROM verification_jobs;
        """)
        job_stats = cur.fetchone()
        print(f"\nVerification Jobs Range:")
        print(f"  Min Job : {job_stats['min_job']}")
        print(f"  Max Job : {job_stats['max_job']}")


if __name__ == "__main__":
    audit_verification_jobs()
    audit_torrents()
    audit_timeline()
