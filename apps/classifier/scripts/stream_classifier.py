import sys
from pathlib import Path
sys.path.append(str(Path(__file__).parent.parent / "src"))
from feature_extractor import TorrentFeatureExtractor
import argparse
import json
import time
from pathlib import Path
import numpy as np
import psycopg2
import joblib

import os
DB_HOST = os.environ.get("DB_HOST", "workspace-production")
PG_PORT = int(os.environ.get("PG_PORT", 5432))
POSTGRES_USER = os.environ.get("POSTGRES_USER", "crawler")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "craw")
PG_PASSWORD = os.environ.get("PG_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b")
MODEL_PATH = Path(__file__).parent.parent / "models" / "torrent_classifier_v2.joblib"

def hex_infohash(bytea_val):
    if bytea_val is None:
        return None
    if isinstance(bytea_val, memoryview):
        return bytea_val.tobytes().hex()
    if isinstance(bytea_val, bytes):
        return bytea_val.hex()
    return str(bytea_val)

def main():
    parser = argparse.ArgumentParser(description="Shared-Nothing Sharded Batch Classifier for 10M+ Torrents")
    parser.add_argument("--batch-size", type=int, default=15000, help="Batch size for DB fetch and vector inference")
    parser.add_argument("--limit", type=int, default=None, help="Max records for this worker (None for all in shard)")
    parser.add_argument("--start-hex", type=str, default="00", help="Shard start hex prefix (e.g. 00)")
    parser.add_argument("--end-hex", type=str, default="ff", help="Shard end hex prefix (e.g. ff)")
    parser.add_argument("--margin-threshold", type=float, default=0.35, help="Margin between top-1 and top-2 proba")
    parser.add_argument("--confidence-threshold", type=float, default=0.65, help="Min probability to accept without review")
    parser.add_argument("--output-jsonl", type=str, default="data/predictions.jsonl", help="Output JSONL filepath")
    args = parser.parse_args()

    print(f"Loading model from {MODEL_PATH}...", flush=True)
    model = joblib.load(MODEL_PATH)
    ext = model['extractor']
    clf = model['classifier']
    classes = np.array(model['classes'])
    print(f"Loaded classifier with {len(classes)} classes: {list(classes)}", flush=True)

    start_boundary = bytes.fromhex(args.start_hex) + b'\x00' * 19
    end_boundary = bytes.fromhex(args.end_hex) + b'\xff' * 19

    print(f"Connecting to {POSTGRES_DB} on {DB_HOST} (READ_ONLY)...", flush=True)
    conn = psycopg2.connect(host=DB_HOST, port=PG_PORT, user=POSTGRES_USER, password=PG_PASSWORD, dbname=POSTGRES_DB)
    conn.set_session(readonly=True)
    cur = conn.cursor()

    out_path = Path(args.output_jsonl)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    if out_path.exists():
        out_path.unlink()

    last_ih = start_boundary
    total_processed = 0
    total_accepted = 0
    total_low_conf = 0
    total_ambiguous = 0
    start_time = time.time()

    print(f"\nWorker started on shard [{args.start_hex}..{args.end_hex}] (Batch size: {args.batch_size})", flush=True)
    print("-" * 85, flush=True)

    try:
        while True:
            fetch_size = args.batch_size
            if args.limit and (total_processed + fetch_size) > args.limit:
                fetch_size = args.limit - total_processed
                if fetch_size <= 0:
                    break

            query = """
                SELECT infohash, name, total_size, file_count, files
                FROM torrents
                WHERE infohash > %s AND infohash <= %s
                ORDER BY infohash
                LIMIT %s;
            """
            cur.execute(query, (last_ih, end_boundary, fetch_size))
            batch_rows = cur.fetchall()
            if not batch_rows:
                break

            last_ih = batch_rows[-1][0]

            records = []
            infohash_hexes = []
            for row in batch_rows:
                ih, name, size, count, files = row
                records.append({
                    "name": name,
                    "total_size": size,
                    "file_count": count,
                    "files": files
                })
                infohash_hexes.append(hex_infohash(ih))

            # Fast vector inference
            X_batch = ext.transform(records)
            probas = clf.predict_proba(X_batch)

            sorted_p = np.sort(probas, axis=1)
            top1_p = sorted_p[:, -1]
            top2_p = sorted_p[:, -2]
            margins = top1_p - top2_p
            pred_indices = probas.argmax(axis=1)
            pred_categories = classes[pred_indices]

            # Dual Rejection Diagnosis
            is_low_conf = (top1_p < args.confidence_threshold)
            is_ambiguous = (~is_low_conf) & (margins < args.margin_threshold)
            is_accepted = (~is_low_conf) & (~is_ambiguous)

            batch_output = []
            for i in range(len(records)):
                if is_accepted[i]:
                    reason = None
                    needs_review = False
                elif is_low_conf[i]:
                    reason = "low_confidence"
                    needs_review = True
                else:
                    reason = "ambiguous_boundary"
                    needs_review = True

                rec_out = {
                    "infohash": infohash_hexes[i],
                    "name": records[i]["name"],
                    "category": pred_categories[i],
                    "confidence": round(float(top1_p[i]), 4),
                    "margin": round(float(margins[i]), 4),
                    "needs_review": needs_review,
                    "review_reason": reason
                }
                batch_output.append(json.dumps(rec_out, ensure_ascii=False))

            with open(out_path, "a", encoding="utf-8") as f:
                f.write("\n".join(batch_output) + "\n")

            batch_cnt = len(records)
            total_processed += batch_cnt
            total_accepted += int(is_accepted.sum())
            total_low_conf += int(is_low_conf.sum())
            total_ambiguous += int(is_ambiguous.sum())

            elapsed = time.time() - start_time
            rate = total_processed / max(elapsed, 0.001)

            print(f"Processed: {total_processed:7d} | Rate: {rate:5.0f} rec/s | Accepted: {total_accepted:7d} ({total_accepted/total_processed*100:4.1f}%) | LowConf: {total_low_conf:6d} ({total_low_conf/total_processed*100:4.1f}%) | Ambig: {total_ambiguous:5d} ({total_ambiguous/total_processed*100:3.1f}%)", flush=True)

            if args.limit and total_processed >= args.limit:
                break

    finally:
        cur.close()
        conn.close()

    total_time = time.time() - start_time
    total_flagged = total_low_conf + total_ambiguous
    print("-" * 85, flush=True)
    print(f"Shard [{args.start_hex}..{args.end_hex}] Complete: {total_processed:,} in {total_time:.1f}s ({total_processed/max(total_time,0.001):.0f} rec/s)", flush=True)
    print(f"Accepted Predictions          : {total_accepted:,} ({total_accepted/max(total_processed,1)*100:.1f}%)", flush=True)
    print(f"Flagged for Review (Total)    : {total_flagged:,} ({total_flagged/max(total_processed,1)*100:.1f}%)", flush=True)
    print(f"  - Low Confidence Sub-Bucket : {total_low_conf:,} ({total_low_conf/max(total_processed,1)*100:.1f}%)", flush=True)
    print(f"  - Ambiguous Boundary Bucket : {total_ambiguous:,} ({total_ambiguous/max(total_processed,1)*100:.1f}%)", flush=True)
    print(f"Saved shard output to: {out_path}", flush=True)

if __name__ == '__main__':
    main()
