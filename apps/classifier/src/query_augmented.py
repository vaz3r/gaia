#!/usr/bin/env python3
"""Query PostgreSQL for additional training examples targeting weak classes."""

import json
import re
import sys

try:
    import psycopg2
except ImportError:
    print("ERROR: psycopg2 required", file=sys.stderr)
    sys.exit(1)

DB_URL = "postgresql://crawler:83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b@workspace-production.tailfe0c2.ts.net:5432/craw?sslmode=disable"

# Get existing training infohashes
existing = set()
with open("data/labeled.jsonl") as f:
    for line in f:
        if line.strip():
            r = json.loads(line)
            existing.add(r["infohash"])
print(f"Existing training infohashes: {len(existing)}")

# Also exclude the fresh sample
fresh = set()
with open("data/fresh_sample.jsonl") as f:
    for line in f:
        if line.strip():
            r = json.loads(line)
            fresh.add(r["infohash"])

exclude = existing | fresh
print(f"Total excluded: {len(exclude)}")

conn = psycopg2.connect(DB_URL)
cur = conn.cursor()

def query_and_save(name_pattern, label, limit=50):
    """Query torrents matching a regex pattern and save as labeled."""
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
      AND t.name ~* %s
      AND t.name IS NOT NULL
      AND t.name != ''
      AND t.name != '[unknown]'
    ORDER BY random()
    LIMIT %s;
    """
    cur.execute(query, (name_pattern, limit))
    
    records = []
    for row in cur:
        r = row[0]
        if r and r.get("infohash") not in exclude:
            r["label_category"] = label
            r["label_keep"] = True
            records.append(r)
            exclude.add(r["infohash"])
    
    print(f"  {label:20s} ({name_pattern[:40]:40s}): {len(records)} new records")
    return records

# Targeted queries for weak classes
new_records = []

print("\n=== Querying PostgreSQL for additional training examples ===\n")

# Documentaries
new_records += query_and_save(r"NOVA|Frontline|PBS|BBC.*(documentary|horizon|storyville)", "Documentaries", 40)
new_records += query_and_save(r"National Geographic|NatGeo|Planet Earth|Blue Planet|Our Planet", "Documentaries", 30)
new_records += query_and_save(r"60 Minutes|Dateline|20/20|investigation|true crime|documentary", "Documentaries", 30)
new_records += query_and_save(r"history.*(channel|documentary)|NHK|discovery channel|wildlife|nature", "Documentaries", 20)

# Games (FitGirl, CODEX, PLAZA, repacks, ISO/NSP with game titles)
new_records += query_and_save(r"FitGirl|FitGirl Repack", "Games", 40)
new_records += query_and_save(r"CODEX|PLAZA|SKIDROW|RUNE|Empress", "Games", 30)
new_records += query_and_save(r"\bNSP\b|\bXCI\b|Nintendo Switch", "Games", 20)
new_records += query_and_save(r"\.ISO\b.*\b(game|PC|windows)", "Games", 20)
new_records += query_and_save(r"repack.*\b(game|PC)", "Games", 20)

# Applications
new_records += query_and_save(r"AutoCAD|Autodesk|Adobe|Photoshop|Premiere|After Effects", "Applications", 30)
new_records += query_and_save(r"Microsoft.*Office|Windows.*activ|Visual Studio", "Applications", 20)
new_records += query_and_save(r"setup\.exe|keygen|crack.*serial|patch.*license", "Applications", 20)
new_records += query_and_save(r"portable.*\.(exe|msi)|v\d+\.\d+\.\d+.*\.(exe|msi)", "Applications", 20)

# Additional anime with fansub markers (to fix anime misclassifications)
new_records += query_and_save(r"\[SubsPlease\]|\[Erai-raws\]|\[HorribleSubs\]", "Anime", 30)

# Additional games with clear markers
new_records += query_and_save(r"Steam|GOG|Epic Games|Battle\.net|EA\b.*app", "Games", 20)

# Television (western, non-anime)
new_records += query_and_save(r"S\d{1,2}E\d{1,3}.*\b(WEB-DL|WEBRip|BluRay)\b", "Television", 30)

conn.close()

# Save new records
with open("data/augmented_training.jsonl", "w") as f:
    for r in new_records:
        f.write(json.dumps(r) + "\n")

print(f"\n=== Total new records: {len(new_records)} ===")
print(f"Saved to data/augmented_training.jsonl")

# Distribution
from collections import Counter
cats = Counter(r["label_category"] for r in new_records)
print("\nNew records by category:")
for cat, count in sorted(cats.items(), key=lambda x: -x[1]):
    print(f"  {cat:20s} {count:4d}")
