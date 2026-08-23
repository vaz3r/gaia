#!/usr/bin/env python3
"""Export fresh torrent sample from PostgreSQL, excluding training infohashes."""

import json
import psycopg2

DB_URL = "postgresql://crawler:83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b@workspace-production.tailfe0c2.ts.net:5432/craw?sslmode=disable"

# Get training infohashes to exclude
training_hashes = set()
with open("data/labeled.jsonl") as f:
    for line in f:
        if line.strip():
            r = json.loads(line)
            training_hashes.add(r["infohash"])
print(f"Excluding {len(training_hashes)} training infohashes")

conn = psycopg2.connect(DB_URL)
cur = conn.cursor()

# Count eligible torrents
cur.execute("""
    SELECT COUNT(*) FROM torrents t
    WHERE t.verified_at IS NOT NULL
      AND t.name IS NOT NULL
      AND t.name != ''
      AND t.name != '[unknown]'
""")
total = cur.fetchone()[0]
print(f"Total eligible verified torrents: {total}")

# Export fresh sample
query = """
SELECT json_build_object(
    'infohash', encode(t.infohash, 'hex'),
    'name', t.name,
    'file_count', t.file_count,
    'total_size', t.total_size,
    'top_dirs', (
        SELECT json_agg(DISTINCT elem)
        FROM (
            SELECT f->>'path' AS elem
            FROM jsonb_array_elements(t.files) f
            WHERE f->>'path' IS NOT NULL
            LIMIT 10
        ) sub
    )
)
FROM torrents t
WHERE t.verified_at IS NOT NULL
  AND t.name IS NOT NULL
  AND t.name != ''
  AND t.name != '[unknown]'
ORDER BY random()
LIMIT 1200;
"""

cur.execute(query)
count = 0
exported = 0
with open("data/fresh_sample.jsonl", "w") as f:
    for row in cur:
        record = row[0]
        count += 1
        if record and record.get("infohash") not in training_hashes:
            f.write(json.dumps(record) + "\n")
            exported += 1
            if exported >= 1000:
                break

cur.close()
conn.close()
print(f"Scanned {count} torrents, exported {exported} fresh ones to data/fresh_sample.jsonl")
