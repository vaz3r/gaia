#!/usr/bin/env python3
import json
import re
import logging
import paramiko
from pathlib import Path

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# Heuristics based on user recommendations
RE_MOVIE_YEAR = re.compile(r'(?:^|[.\s_-])(19\d{2}|20\d{2})(?:[.\s_-]|$)')
RE_VIDEO_EXT = re.compile(r'\.(mkv|mp4|avi|wmv)(?:[.\s_-]|$)', re.IGNORECASE)
RE_EPISODE = re.compile(r'(?:^|[.\s_-])(?:S\d{2}E\d{2}|Season)(?:[.\s_-]|$)', re.IGNORECASE)
RE_ANIME = re.compile(r'\[(?:SubsPlease|Erai-raws|HorribleSubs|Judas|Cleo|Beatrice-Raws)\]', re.IGNORECASE)
RE_ADULT = re.compile(
    r'(?:^|[.\s_-])(?:jav|javguru|caribbeancom|tokyo-hot|heyzo|1pondo|xxx|onlyfans|hentai|porn|blowjob|cumshot|anal|creampie|webcam|chaturbate|camsoda)(?:[.\s_-]|$)|'
    r'(?:^|[.\s_-])(?:brazzers|bangbros|naughty|evilangel|realitykings|mofos|x-art|tushy|blacked|vixen|milf|teenslovehugecocks|hustler|peter north|bigtits)(?:[.\s_-]|$)|'
    r'(?<![A-Z])[A-Z]{2,5}-\d{2,5}(?!\d)|'
    r'\[(?:JAV|Hentai|Adult|Ecchi)\]', 
    re.IGNORECASE
)
RE_GAME = re.compile(r'(?:^|[.\s_-])(?:FitGirl|CODEX|PLAZA|REPACK|ISO|NSP|XCI|crack|GOG|Steam)(?:[.\s_-]|$)', re.IGNORECASE)
RE_MUSIC_EXT = re.compile(r'\.(flac|mp3)(?:[.\s_-]|$)', re.IGNORECASE)
RE_MUSIC_HINT = re.compile(r'Cover\.jpg', re.IGNORECASE)
RE_APP = re.compile(r'(?:^|[.\s_-])(?:setup\.exe|keygen|portable|v\d+\.\d+)(?:[.\s_-]|$)', re.IGNORECASE)
RE_DOCU = re.compile(r'(?:^|[.\s_-])(?:BBC|PBS|NOVA|Frontline|NatGeo|documentary)(?:[.\s_-]|$)', re.IGNORECASE)
RE_OTHER_JUNK = re.compile(r'(?:^|[.\s_-])(?:password|malware|www\.|http:|https:)(?:[.\s_-]|$)', re.IGNORECASE)

def extract_text_signals(name, files_list):
    text = name + " " + " ".join(files_list)
    has_year = bool(RE_MOVIE_YEAR.search(name)) # usually year is in name
    has_vid = bool(RE_VIDEO_EXT.search(text))
    has_ep = bool(RE_EPISODE.search(text))
    has_anime = bool(RE_ANIME.search(text))
    has_adult = bool(RE_ADULT.search(text))
    has_game = bool(RE_GAME.search(text))
    has_music_ext = bool(RE_MUSIC_EXT.search(text))
    has_music_hint = bool(RE_MUSIC_HINT.search(text))
    has_app = bool(RE_APP.search(text))
    has_docu = bool(RE_DOCU.search(text))
    has_junk = bool(RE_OTHER_JUNK.search(text))
    
    return {
        "year": has_year,
        "vid": has_vid,
        "ep": has_ep,
        "anime": has_anime,
        "adult": has_adult,
        "game": has_game,
        "music_ext": has_music_ext,
        "music_hint": has_music_hint,
        "app": has_app,
        "docu": has_docu,
        "junk": has_junk
    }

def get_weak_label(signals):
    candidates = []
    
    if signals["year"] and signals["vid"] and not signals["ep"] and not signals["anime"]:
        candidates.append("Movies")
        
    if signals["ep"] and not signals["anime"] and not signals["adult"]:
        candidates.append("Television")
        
    if signals["game"] and not (signals["vid"] and signals["year"]):
        candidates.append("Games")
        
    if signals["music_ext"] and not signals["vid"]:
        candidates.append("Music")
        
    if signals["app"] and not signals["vid"] and not signals["music_ext"]:
        candidates.append("Applications")
        
    if signals["anime"]:
        candidates.append("Anime")
        
    if signals["docu"]:
        candidates.append("Documentaries")
        
    if signals["adult"] or signals["junk"]:
        candidates.append("Other")
        
    if len(candidates) == 1:
        return candidates[0]
        
    if len(candidates) == 0 and (signals["adult"] or signals["junk"]):
        return "Other"
        
    return None

def main():
    logger.info("Connecting to workspace-production via SSH...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect("workspace-production", username="core", password="rosrtdz@1995")
    
    query = """
    COPY (
        SELECT row_to_json(t) FROM (
            SELECT encode(infohash, 'hex') as infohash, name, file_count, total_size as total_size_bytes, 
                   COALESCE(
                       (SELECT jsonb_agg(f->>'path') FROM jsonb_array_elements(files) as f),
                       '[]'::jsonb
                   ) as top_dirs
            FROM torrents TABLESAMPLE SYSTEM(30)
            LIMIT 150000
        ) t
    ) TO STDOUT;
    """
    cmd = f"docker exec craw-db psql -U crawler -d craw -c \"{query}\""
    
    logger.info("Executing remote query...")
    stdin, stdout, stderr = client.exec_command(cmd)
    
    output_file = Path("data/weak_labeled.jsonl")
    count_total = 0
    count_labeled = 0
    labels = {c: 0 for c in ["Movies", "Television", "Games", "Music", "Applications", "Anime", "Documentaries", "Other"]}
    
    with open(output_file, "w", encoding="utf-8") as f_out:
        for line in stdout:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
                
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
                # the label is saved as label_category to be consistent with train dataset
                row["label_category"] = label
                row["method"] = "weak_heuristics"
                f_out.write(json.dumps(row) + "\n")
                count_labeled += 1
                labels[label] += 1
                
            if count_total % 10000 == 0:
                logger.info(f"Scanned {count_total}, labeled {count_labeled}...")
                
    client.close()
    
    logger.info(f"Done! Scanned {count_total}, labeled {count_labeled} total.")
    for k, v in labels.items():
        logger.info(f"  {k}: {v}")
    logger.info(f"Wrote to {output_file}")

if __name__ == "__main__":
    main()
