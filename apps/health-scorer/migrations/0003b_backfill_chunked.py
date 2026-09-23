#!/usr/bin/env python3
"""
0003b_backfill_chunked.py — Keyset-paginated backfill of health_scores from torrents table.

Safely populates canonical health_scores for all existing torrents in chunks of 50,000,
avoiding long exclusive table locks and excessive WAL pressure.
Idempotent: uses ON CONFLICT (infohash) DO NOTHING.
"""

import os
import sys
import time
import psycopg2

DB_HOST = os.environ.get("DB_HOST", "localhost")
PG_PORT = int(os.environ.get("PG_PORT", 5432))
POSTGRES_USER = os.environ.get("POSTGRES_USER", "crawler")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "craw")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b")

CHUNK_SIZE = int(os.environ.get("CHUNK_SIZE", 50000))

def main():
    print(f"Connecting to {POSTGRES_USER}@{DB_HOST}:{PG_PORT}/{POSTGRES_DB}...")
    conn = psycopg2.connect(
        host=DB_HOST,
        port=PG_PORT,
        user=POSTGRES_USER,
        password=PG_PASSWORD,
        dbname=POSTGRES_DB,
    )
    conn.autocommit = True

    cursor_hex = None
    total_inserted = 0
    chunk_index = 0
    t0 = time.time()

    query_first = """
        INSERT INTO health_scores (infohash, health_score, health_state, confidence, algorithm_version, health_calculated_at)
        SELECT
            encode(t.infohash, 'hex'),
            t.health_score,
            CASE
                WHEN t.health_score IS NULL THEN 'UNKNOWN'
                WHEN t.health_score >= 70 THEN 'VERIFIED'
                WHEN t.health_score >= 25 THEN 'UNVERIFIED'
                WHEN t.health_score > 0 THEN 'STALE'
                ELSE 'UNKNOWN'
            END,
            CASE
                WHEN t.health_score IS NULL THEN 0.0
                WHEN t.health_score >= 70 THEN 0.8
                WHEN t.health_score >= 25 THEN 0.5
                WHEN t.health_score > 0 THEN 0.3
                ELSE 0.0
            END,
            'backfill_v1',
            NOW()
        FROM (
            SELECT infohash, health_score
            FROM torrents
            ORDER BY infohash ASC
            LIMIT %s
        ) t
        ON CONFLICT (infohash) DO NOTHING
        RETURNING infohash;
    """

    query_next = """
        INSERT INTO health_scores (infohash, health_score, health_state, confidence, algorithm_version, health_calculated_at)
        SELECT
            encode(t.infohash, 'hex'),
            t.health_score,
            CASE
                WHEN t.health_score IS NULL THEN 'UNKNOWN'
                WHEN t.health_score >= 70 THEN 'VERIFIED'
                WHEN t.health_score >= 25 THEN 'UNVERIFIED'
                WHEN t.health_score > 0 THEN 'STALE'
                ELSE 'UNKNOWN'
            END,
            CASE
                WHEN t.health_score IS NULL THEN 0.0
                WHEN t.health_score >= 70 THEN 0.8
                WHEN t.health_score >= 25 THEN 0.5
                WHEN t.health_score > 0 THEN 0.3
                ELSE 0.0
            END,
            'backfill_v1',
            NOW()
        FROM (
            SELECT infohash, health_score
            FROM torrents
            WHERE infohash > decode(%s, 'hex')
            ORDER BY infohash ASC
            LIMIT %s
        ) t
        ON CONFLICT (infohash) DO NOTHING
        RETURNING infohash;
    """

    cursor_finder = """
        SELECT encode(infohash, 'hex')
        FROM torrents
        WHERE infohash > decode(%s, 'hex')
        ORDER BY infohash ASC
        LIMIT 1 OFFSET %s;
    """

    cursor_finder_first = """
        SELECT encode(infohash, 'hex')
        FROM torrents
        ORDER BY infohash ASC
        LIMIT 1 OFFSET %s;
    """

    print("Starting chunked backfill...")
    with conn.cursor() as cur:
        while True:
            chunk_t0 = time.time()
            chunk_index += 1

            if cursor_hex is None:
                cur.execute(query_first, (CHUNK_SIZE,))
            else:
                cur.execute(query_next, (cursor_hex, CHUNK_SIZE))

            inserted_in_chunk = cur.rowcount
            total_inserted += inserted_in_chunk

            # Find next cursor boundary (infohash at CHUNK_SIZE offset)
            if cursor_hex is None:
                cur.execute(cursor_finder_first, (CHUNK_SIZE - 1,))
            else:
                cur.execute(cursor_finder, (cursor_hex, CHUNK_SIZE - 1))

            row = cur.fetchone()
            if not row:
                print(f"Chunk {chunk_index}: reached end of torrents table. Done!")
                break

            cursor_hex = row[0]
            chunk_dur = time.time() - chunk_t0
            rate = total_inserted / max(time.time() - t0, 0.001)
            print(f"Chunk {chunk_index:3d}: cursor={cursor_hex[:12]}... inserted={inserted_in_chunk:6d} (total={total_inserted:7d}) in {chunk_dur:.2f}s [{rate:.0f} rows/s]")

    elapsed = time.time() - t0
    print(f"\nBackfill completed! Total new rows inserted: {total_inserted} in {elapsed:.1f}s")
    conn.close()

if __name__ == "__main__":
    main()
