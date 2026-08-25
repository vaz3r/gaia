import json
import sys
import re

def categorize(name, files):
    name = name.lower()
    files_str = " ".join([f.lower() for f in files]) if files else ""
    full_text = name + " " + files_str

    if re.search(r'\b(rj\d+|dlsite|jav|censored|uncensored|fc2|xxx|porn|hentai|fanza|adult|loli|家庭内)\b', full_text):
        return "Porn"
    if re.search(r'(同人|cg集|エロ|成年|成年コミック|fap)', full_text):
        return "Porn"
    if re.search(r'\[(ai generated|patreon|nonsummerjack|djawa|ein)\]', name.lower()) or "ai generated" in full_text:
        return "Porn"

    if re.search(r'\b(s\d{2}e\d{2}|season \d+)\b', full_text):
        return "Television"

    if re.search(r'\b(crack|repack|fitgirl|dodi|skidrow|codex|plaza|cpy|tenoke|rune)\b', full_text) or any(f.lower().endswith('.iso') for f in (files or [])):
        if "need_for_speed" in name or "osu!" in name:
             return "Games"
        return "Games"
    if re.search(r'(need_for_speed|osu!|tycoon|adventures|ravenfield|openrct|build\.\d+|kathyrain2|demeo\.x)', name):
        return "Games"

    if re.search(r'\b(1080p|720p|2160p|bluray|brrip|web-dl|webrip)\b', full_text):
         if "episode" in full_text or re.search(r'\[[a-zA-Z0-9_-]+\]', name):
             return "Anime"
         return "Movies"

    if re.search(r'(\.mp3|\.flac|\.wav)', files_str) or "discography" in full_text or "ost" in full_text.split() or "320k mp3" in full_text:
        return "Music"

    if re.search(r'(\.exe|\.dmg|\.apk|macos|windows 10|office 20)', full_text):
        return "Applications"

    if re.search(r'(\.mkv|\.mp4|\.avi)', files_str):
        if re.search(r'\[.*\]', name):
             return "Anime"
        return "Movies"

    if re.search(r'(manga|dlraw|raw|comic|vol \d+|chapter|japariket|suzumiya haruhi)', full_text):
        return "Anime"

    # Manga/Doujin markers
    if re.search(r'(\(c\d+\)|\(コミティア\d+\)|\[.*\]\s*\[.*\]|artbook|\b(spanish|italian|chinese|korean|english|digital)\b|汉化)', full_text):
        return "Anime"

    if re.search(r'(v\d+\.\d+\.\d+|edition)', full_text):
        return "Games" 

    return "Other"

with open("to_label.jsonl", "r") as f, open("labeled_150.jsonl", "w") as out:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            data = json.loads(line)
        except Exception as e:
            continue
        data["label_category"] = categorize(data["name"], data.get("files"))
        out.write(json.dumps(data) + "\n")
