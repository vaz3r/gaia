import json
import os
import re

data_dir = '/Users/omega/Documents/GitHub/gaia/apps/classifier/data'
edge_cases_path = os.path.join(data_dir, 'edge_cases_large.jsonl')
labeled_path = os.path.join(data_dir, 'labeled.jsonl')

def label_heuristic(item):
    name = item.get('name', '').lower()
    files_str = json.dumps(item.get('files', {})).lower()
    
    # 1. Games (Repacks, cracked scenes)
    if any(x in name for x in ['fitgirl', 'repack', 'dodi', 'plaza', 'skidrow', 'codex', 'r.g. mechanics']):
        return 'Games'
        
    # 2. Movies / TV (x264, 1080p, 720p, bluray)
    # Distinguishing Movies and TV is hard. Usually TV has S01E01 patterns.
    if re.search(r's\d{2}e\d{2}', name):
        return 'Television'
    if any(x in name for x in ['1080p', '720p', 'x264', 'bluray', 'bdrip', 'web-dl', 'webrip']):
        return 'Movies'
        
    # 3. Music (FLAC, MP3, 320kbps)
    if any(x in name for x in ['flac', '320kbps', 'v0']) or 'mp3' in name:
        return 'Music'
        
    # 4. Anime (Subs, Dubs)
    if any(x in name for x in ['[subs]', '[dub]', 'horriblesubs', 'erai-raws']):
        return 'Anime'
        
    return None

existing_hashes = set()
with open(labeled_path, 'r') as f:
    for line in f:
        data = json.loads(line)
        if 'infohash' in data:
            existing_hashes.add(data['infohash'])

labeled_count = 0
with open(edge_cases_path, 'r') as f, open(labeled_path, 'a') as out:
    for line in f:
        line = line.strip()
        if not line.startswith('{'): continue
        try:
            data = json.loads(line)
            infohash = data.get('infohash')
            if infohash and infohash not in existing_hashes:
                label = label_heuristic(data)
                if label:
                    # Clean up DB response
                    data['label_category'] = label
                    out.write(json.dumps(data) + '\n')
                    existing_hashes.add(infohash)
                    labeled_count += 1
        except Exception as e:
            pass

print(f"Successfully auto-labeled and appended {labeled_count} edge cases to the training set using strict heuristics.")
