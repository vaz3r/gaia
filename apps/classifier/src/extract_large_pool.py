#!/usr/bin/env python3
"""
Extract a large (60k+) deduplicated unlabeled torrent pool from PostgreSQL.
Excludes all hashes from training, validation, and test datasets.
"""
import json
import logging
from pathlib import Path
import paramiko

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

EXCLUDE_FILES = [
    "data/manual_eval_set_1000.jsonl",
    "data/manual_eval_set_200.jsonl",
    "data/training_combined_v10_true.jsonl",
    "data/manual_seed_1800.csv",
    "data/val_v10_20pct.jsonl",
    "data/train_v10_80pct.jsonl",
]

def load_excluded_hashes():
    excluded = set()
    for fpath in EXCLUDE_FILES:
        p = Path(fpath)
        if not p.exists():
            continue
        with open(p, "r", encoding="utf-8") as f:
            for line in f:
                if not line.strip():
                    continue
                try:
                    if fpath.endswith(".csv"):
                        # infohash,label_category
                        parts = line.strip().split(",")
                        if len(parts) >= 1 and len(parts[0]) >= 16:
                            excluded.add(parts[0].strip().lower())
                    else:
                        row = json.loads(line)
                        ih = row.get("infohash", row.get("id", ""))
                        if ih:
                            excluded.add(ih.strip().lower())
                except Exception:
                    pass
    logger.info("Loaded %d excluded infohashes across all existing datasets.", len(excluded))
    return excluded

def extract_pool(target_count=60000, output_path="data/unlabeled_pool_60k.jsonl"):
    excluded = load_excluded_hashes()
    
    logger.info("Connecting to workspace-production via SSH...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect("workspace-production", username="core", password="rosrtdz@1995", timeout=10)
    
    # Query ~100k rows using TABLESAMPLE to comfortably get 60k clean items
    query = """
    COPY (
        SELECT row_to_json(t) FROM (
            SELECT encode(infohash, 'hex') as infohash, name, file_count, total_size as total_size_bytes, 
                   COALESCE(
                       (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                       '[]'::jsonb
                   ) as top_dirs
            FROM torrents TABLESAMPLE SYSTEM(18)
            LIMIT 100000
        ) t
    ) TO STDOUT;
    """
    cmd = f'docker exec craw-db psql -U crawler -d craw -c "{query}"'
    
    logger.info("Executing remote stream query...")
    stdin, stdout, stderr = client.exec_command(cmd, bufsize=65536)
    
    out_file = Path(output_path)
    count_saved = 0
    seen_in_batch = set()
    
    with open(out_file, "w", encoding="utf-8") as f_out:
        for line in stdout:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except Exception:
                continue
            
            ih = row.get("infohash", "").strip().lower()
            name = row.get("name", "").strip()
            
            if not ih or not name:
                continue
            if ih in excluded or ih in seen_in_batch:
                continue
            
            seen_in_batch.add(ih)
            f_out.write(json.dumps(row) + "\n")
            count_saved += 1
            
            if count_saved % 10000 == 0:
                logger.info("Extracted %d / %d clean torrents...", count_saved, target_count)
            
            if count_saved >= target_count:
                break
                
    client.close()
    logger.info("Successfully extracted %d clean torrents to %s", count_saved, out_file)
    return count_saved

if __name__ == "__main__":
    extract_pool()
