import json
import os

data_dir = '/Users/omega/Documents/GitHub/gaia/apps/classifier/data'
labeled_path = os.path.join(data_dir, 'labeled.jsonl')
temp_path = os.path.join(data_dir, 'labeled.jsonl.tmp')

fixed_count = 0
with open(labeled_path, 'r') as f, open(temp_path, 'w') as out:
    for line in f:
        data = json.loads(line)
        files = data.get('files')
        
        # files might already be partially fixed, or it might be lists of lists
        if isinstance(files, list) and len(files) > 0:
            new_files = []
            changed = False
            for f_item in files:
                if isinstance(f_item, dict):
                    path = f_item.get('path', '')
                    if isinstance(path, list):
                        new_files.append('/'.join(path))
                    else:
                        new_files.append(str(path))
                    changed = True
                elif isinstance(f_item, list):
                    new_files.append('/'.join(str(x) for x in f_item))
                    changed = True
                else:
                    new_files.append(str(f_item))
            if changed or any(not isinstance(x, str) for x in files):
                data['files'] = new_files
                fixed_count += 1
            
        out.write(json.dumps(data) + '\n')

os.replace(temp_path, labeled_path)
print(f"Fixed {fixed_count} records in labeled.jsonl")
