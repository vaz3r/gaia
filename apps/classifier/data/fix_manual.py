import json
import os

def fix_file(filepath):
    print(f"Fixing {filepath}...")
    temp_filepath = filepath + ".tmp"
    changed = 0
    keywords = [
        "Casey Calvert", "Bella.Diamond", "Hustler.com", "SubbyHubby.com", 
        "TeensLoveHugeCocks", "Faces of Ecstasy", "Alaura Eden", "Chica.Boom",
        "Adult", "BigTits", "HugeCocks"
    ]
    with open(filepath, 'r') as infile, open(temp_filepath, 'w') as outfile:
        for line in infile:
            data = json.loads(line)
            name = data.get("name", "")
            
            label_field = None
            if "label_category" in data:
                label_field = "label_category"
            elif "true_category" in data:
                label_field = "true_category"
            elif "label" in data:
                label_field = "label"
                
            if label_field and data[label_field] == "Other":
                if any(kw.lower() in name.lower() for kw in keywords):
                    data[label_field] = "Porn"
                    changed += 1
            
            outfile.write(json.dumps(data) + '\n')
            
    os.replace(temp_filepath, filepath)
    print(f"Fixed {changed} records in {filepath}")

def main():
    data_dir = "/Users/omega/Documents/GitHub/gaia/apps/classifier/data"
    fix_file(os.path.join(data_dir, "labeling_sample_final.jsonl"))
    fix_file(os.path.join(data_dir, "labeled.jsonl"))

if __name__ == "__main__":
    main()
