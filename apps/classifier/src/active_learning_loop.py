#!/usr/bin/env python3
import json
import logging
import argparse
from pathlib import Path

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def main():
    parser = argparse.ArgumentParser(description="Active Learning Selection")
    parser.add_argument("--predictions", required=True, help="JSONL file with predictions")
    parser.add_argument("--pool", required=True, help="JSONL file with full metadata pool")
    parser.add_argument("--output", required=True, help="Output CSV for manual labeling")
    parser.add_argument("--n", type=int, default=500, help="Number of samples to extract")
    args = parser.parse_args()
    
    # Load metadata pool
    logger.info(f"Loading metadata from {args.pool}...")
    metadata = {}
    with open(args.pool, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip(): continue
            try:
                row = json.loads(line)
                if "infohash" in row:
                    metadata[row["infohash"]] = row
            except:
                pass

    logger.info(f"Loading predictions from {args.predictions}...")
    candidates = []
    
    with open(args.predictions, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip(): continue
            try:
                row = json.loads(line)
                top_cands = row.get("top_candidates", [])
                if not top_cands: continue
                
                # Uncertainty = 1.0 - top_confidence
                top_conf = top_cands[0]["confidence"]
                uncertainty = 1.0 - top_conf
                
                candidates.append((uncertainty, row))
            except json.JSONDecodeError:
                pass
                
    # Sort by highest uncertainty
    candidates.sort(key=lambda x: x[0], reverse=True)
    
    top_candidates = [c[1] for c in candidates[:args.n]]
    
    logger.info(f"Writing top {args.n} uncertain samples to {args.output}...")
    import csv
    with open(args.output, "w", newline="", encoding="utf-8") as f_out:
        writer = csv.writer(f_out)
        writer.writerow(["infohash", "name", "file_count", "total_size_bytes", "top_dirs", "model_prediction", "uncertainty", "TRUE_LABEL"])
        for row in top_candidates:
            ih = row.get("infohash")
            meta = metadata.get(ih, {})
            uncertainty_val = next(c[0] for c in candidates if c[1]["infohash"] == ih)
            
            top_dirs = meta.get("top_dirs", [])
            # Flatten lists if they are lists
            str_dirs = []
            for d in top_dirs:
                if isinstance(d, list):
                    str_dirs.append("/".join(str(p) for p in d))
                else:
                    str_dirs.append(str(d))
            top_dirs_str = " | ".join(str_dirs[:5])
            
            writer.writerow([
                ih,
                meta.get("name", ""),
                meta.get("file_count", 0),
                meta.get("total_size_bytes", 0),
                top_dirs_str,
                row.get("category"),
                f"{uncertainty_val:.3f}",
                ""
            ])
            
    logger.info("Done.")

if __name__ == "__main__":
    main()
