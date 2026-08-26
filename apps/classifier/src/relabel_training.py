#!/usr/bin/env python3
import json
import logging
import sys
from pathlib import Path

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def main():
    input_file = Path("data/labeled.jsonl")
    output_file = Path("data/labeled_8way.jsonl")

    if not input_file.exists():
        logger.error(f"Input file not found: {input_file}")
        sys.exit(1)

    count_total = 0
    count_remapped = 0

    with open(input_file, "r", encoding="utf-8") as f_in, \
         open(output_file, "w", encoding="utf-8") as f_out:
        for line in f_in:
            if not line.strip():
                continue
            data = json.loads(line)
            count_total += 1
            if data.get("label_category") == "Porn":
                data["label_category"] = "Other"
                count_remapped += 1
            f_out.write(json.dumps(data) + "\n")

    logger.info(f"Processed {count_total} total records.")
    logger.info(f"Remapped {count_remapped} 'Porn' records to 'Other'.")
    logger.info(f"Wrote 8-way labeled data to {output_file}")

if __name__ == "__main__":
    main()
