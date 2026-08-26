#!/usr/bin/env python3
import json
import logging
from pathlib import Path
from extract_weak_labels import extract_text_signals, get_weak_label

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def main():
    input_file = Path("data/natural_test_set_2000.jsonl")
    output_file = Path("data/labeled_natural_test_set.jsonl")
    
    count_total = 0
    count_labeled = 0
    
    with open(input_file, "r", encoding="utf-8") as f_in, \
         open(output_file, "w", encoding="utf-8") as f_out:
        for line in f_in:
            if not line.strip(): continue
            row = json.loads(line)
            count_total += 1
            
            name = row.get("name") or ""
            files = row.get("top_dirs") or []
            signals = extract_text_signals(name, files)
            label = get_weak_label(signals)
            
            if not label and sum(signals.values()) == 0:
                import random
                if random.random() < 0.1:
                    label = "Other"
            
            if label:
                row["label_category"] = label
            else:
                row["label_category"] = "Other" # fallback for test set evaluation
                
            f_out.write(json.dumps(row) + "\n")
            count_labeled += 1
            
    logger.info(f"Labeled {count_labeled} / {count_total} in {output_file}")

if __name__ == "__main__":
    main()
