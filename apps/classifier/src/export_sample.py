#!/usr/bin/env python3
"""One-time sample exporter from PostgreSQL (read-only)."""

from __future__ import annotations

import argparse
import json
import sys


def export_sample(db_url: str, output_path: str, limit: int = 1000):
    try:
        import psycopg2
    except ImportError:
        print("ERROR: psycopg2 required. Install with: pip install psycopg2-binary", file=sys.stderr)
        sys.exit(1)

    conn = psycopg2.connect(db_url)
    cur = conn.cursor()

    query = """
    SELECT json_build_object(
        'infohash', encode(t.infohash, 'hex'),
        'name', t.name,
        'file_count', t.file_count,
        'total_size', t.total_size,
        'top_dirs', (
            SELECT json_agg(DISTINCT (f->'path'->>0))
            FROM jsonb_array_elements(t.files) f
            WHERE jsonb_typeof(f->'path') = 'array'
        )
    )
    FROM torrents t
    WHERE t.name IS NOT NULL AND t.name <> '' AND t.name <> '[unknown]'
    ORDER BY random()
    LIMIT %s;
    """

    cur.execute(query, (limit,))
    count = 0

    with open(output_path, "w", encoding="utf-8") as f:
        for row in cur:
            f.write(json.dumps(row[0]) + "\n")
            count += 1

    cur.close()
    conn.close()

    print(f"Exported {count} torrents to {output_path}")


def main():
    parser = argparse.ArgumentParser(description="Export torrent sample from PostgreSQL")
    parser.add_argument("--db", required=True, help="PostgreSQL connection URL")
    parser.add_argument("--output", required=True, help="Output JSONL file")
    parser.add_argument("--limit", type=int, default=1000, help="Number of samples")
    args = parser.parse_args()

    export_sample(args.db, args.output, args.limit)


if __name__ == "__main__":
    main()
