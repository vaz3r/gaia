import json
import re
import os
import glob

PORN_PATTERNS = [
    r'\b(?:jav|javguru|caribbeancom|tokyo-hot|heyzo|1pondo|xxx|onlyfans|hentai|porn|blowjob|cumshot|anal|creampie|webcam|chaturbate|camsoda|adult)\b',
    r'\b[A-Z]{2,5}-\d{2,5}\b',  # JAV code
    r'\[(?:JAV|Hentai|Adult|Ecchi)\]',  # explicit tag
]
regexes = [re.compile(p, re.IGNORECASE) for p in PORN_PATTERNS]

def is_porn(text):
    for r in regexes:
        if r.search(text):
            return True
    return False

def remap_category(category, name, top_dirs):
    if category != "Unwanted":
        return category
    
    combined_text = name + " " + " ".join(top_dirs)
    if is_porn(combined_text):
        return "Porn"
    return "Other"

def process_jsonl(filepath):
    print(f"Processing {filepath}...")
    temp_filepath = filepath + ".tmp"
    changed_count = 0
    total_count = 0
    with open(filepath, 'r') as infile, open(temp_filepath, 'w') as outfile:
        for line in infile:
            total_count += 1
            data = json.loads(line)
            
            # Check for different label fields
            label_field = None
            if "label_category" in data:
                label_field = "label_category"
            elif "true_category" in data:
                label_field = "true_category"
            elif "label" in data:
                label_field = "label"
                
            if label_field and data[label_field] == "Unwanted":
                name = data.get("name", "")
                top_dirs = data.get("top_dirs", [])
                if isinstance(top_dirs, str):
                    top_dirs = [top_dirs]
                
                new_cat = remap_category(data[label_field], name, top_dirs)
                if new_cat != data[label_field]:
                    data[label_field] = new_cat
                    changed_count += 1
            
            outfile.write(json.dumps(data) + '\n')
            
    os.replace(temp_filepath, filepath)
    print(f"  Updated {changed_count}/{total_count} records.")

def process_label_map(filepath):
    print(f"Processing {filepath}...")
    temp_filepath = filepath + ".tmp"
    with open(filepath, 'r') as infile, open(temp_filepath, 'w') as outfile:
        for line in infile:
            # We don't have text context for label_map easily without loading the jsonl,
            # but we can try to rely on jsonl relabeling or just change them to 'Other' and let manual review handle it.
            # However, label_map just maps indices to categories. We'd have to parse labeled.jsonl simultaneously.
            # For simplicity, we'll replace 'Unwanted' in label_map with 'Porn' as a default, 
            # OR we can just write a separate logic if needed. Let's just do a naive replace to 'Other' for now and print a warning.
            
            if "'Unwanted'" in line:
                line = line.replace("'Unwanted'", "'Other'")
            if "Unwanted" in line and "# Taxonomy" in line:
                line = line.replace("Unwanted", "Porn")
            
            outfile.write(line)
    os.replace(temp_filepath, filepath)
    print("  Note: label_map.py Unwanted replaced with Other. Manual review of label_map may be needed.")

def main():
    data_dir = os.path.dirname(os.path.abspath(__file__))
    jsonl_files = glob.glob(os.path.join(data_dir, "*.jsonl"))
    
    for f in jsonl_files:
        process_jsonl(f)
        
    label_map_path = os.path.join(data_dir, "label_map.py")
    if os.path.exists(label_map_path):
        process_label_map(label_map_path)
        
if __name__ == "__main__":
    main()
