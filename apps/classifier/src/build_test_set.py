#!/usr/bin/env python3
import json
import logging
import paramiko
from pathlib import Path

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def load_excluded_hashes(files):
    excluded = set()
    for fpath in files:
        if not Path(fpath).exists():
            continue
        with open(fpath, "r", encoding="utf-8") as f:
            for line in f:
                if not line.strip(): continue
                try:
                    row = json.loads(line)
                    if "infohash" in row:
                        excluded.add(row["infohash"])
                except:
                    pass
    return excluded

def main():
    excluded = load_excluded_hashes(["data/labeled_8way.jsonl", "data/weak_labeled.jsonl"])
    logger.info(f"Loaded {len(excluded)} excluded infohashes from training sets.")
    
    logger.info("Connecting to workspace-production via SSH...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect("workspace-production", username="core", password="rosrtdz@1995")
    
    # Query 10000 random to ensure we get 2000 after exclusion
    query = """
    COPY (
        SELECT row_to_json(t) FROM (
            SELECT encode(infohash, 'hex') as infohash, name, file_count, total_size as total_size_bytes, 
                   COALESCE(
                       (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                       '[]'::jsonb
                   ) as top_dirs
            FROM torrents TABLESAMPLE SYSTEM(5)
            LIMIT 10000
        ) t
    ) TO STDOUT;
    """
    cmd = f"docker exec craw-db psql -U crawler -d craw -c \"{query}\""
    
    logger.info("Executing remote query for natural test set...")
    stdin, stdout, stderr = client.exec_command(cmd)
    
    output_file = Path("data/natural_test_set_2000.jsonl")
    count_saved = 0
    
    with open(output_file, "w", encoding="utf-8") as f_out:
        for line in stdout:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except:
                continue
                
            ih = row.get("infohash")
            if not ih or ih in excluded:
                continue
                
            f_out.write(json.dumps(row) + "\n")
            count_saved += 1
            if count_saved >= 2000:
                break
                
    client.close()
    logger.info(f"Saved {count_saved} natural test set samples to {output_file}.")

if __name__ == "__main__":
    main()
