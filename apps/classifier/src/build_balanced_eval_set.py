#!/usr/bin/env python3
"""
Build a balanced evaluation benchmark dataset from PostgreSQL.
Target: ~1,600 - 2,000 samples with strong representation across all 8 classes:
- Games: ~100
- Applications: ~100
- Documentaries: ~100
- Music: ~150
- Anime: ~200
- Movies: ~300
- Television: ~300
- Other: ~500
"""
import json
import logging
import re
from collections import Counter, defaultdict
from pathlib import Path
import paramiko

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

EXCLUDE_FILES = [
    "data/manual_eval_set_1000.jsonl",
    "data/manual_eval_set_200.jsonl",
    "data/training_combined_v10_true.jsonl",
    "data/al_labeled_1129_true.jsonl",
    "data/pseudo_labels_pool.jsonl",
    "data/training_semi_supervised_v1.jsonl",
]

def load_excluded():
    excluded = set()
    for fpath in EXCLUDE_FILES:
        p = Path(fpath)
        if not p.exists():
            continue
        with open(p, "r", encoding="utf-8") as f:
            for line in f:
                if not line.strip():
                    continue
                try:
                    row = json.loads(line)
                    ih = row.get("infohash", row.get("id", "")).strip().lower()
                    if ih:
                        excluded.add(ih)
                except Exception:
                    pass
    logger.info("Loaded %d excluded infohashes.", len(excluded))
    return excluded


# ── Domain Pattern Classifiers for Gold Labeling ─────────────────────────────
ADULT = re.compile(r'\b(xxx|porn|sex|anal|milf|hardcore|creampie|brazzers|bangbros|hentai|nude|fetish|gangbang|blowjob|nsfw|lewd|onlyfans|cumshot|squirt|bdsm|bondage|lesbian|threesome|jav|fc2)\b', re.IGNORECASE)
GAME_K = re.compile(r'\b(fitgirl|codex|plaza|repack|skidrow|rune|empress|tenoke|dodi|gog|cpath|flt|razor1911|reloaded|cpy|goldberg|steamrip|kaos|elamigos|xatab)\b', re.IGNORECASE)
GAME_EXT = re.compile(r'\.(nsp|xci|pkg|vpk|cia|rom|nds|3ds|gcm|ciso|wbfs)\b', re.IGNORECASE)
APP_K = re.compile(r'setup\.exe|keygen|portable|activator|patcher|crack\.exe|\.msi\b|\.dmg\b|serial\.txt', re.IGNORECASE)
KNOWN_SW = re.compile(r'\b(adobe|microsoft|office|autocad|vmware|photoshop|premiere|illustrator|acrobat|autodesk|corel|matlab|solidworks|visual.?studio|windows\s*\d+|win\d+|winrar|7-zip|vlc|blender|davinci.?resolve|ableton|fl.?studio|cubase|protools|kaspersky|norton|avast|malwarebytes|ccleaner)\b', re.IGNORECASE)
DOC_K = re.compile(r'\b(bbc|pbs|nova\b|frontline|natgeo|national.?geographic|documentary|nhk|biography|history.?channel|discovery.?channel|curiositystream|nature.?pbs|panorama|horizon.?bbc|storyville|disneynature|attenborough|louis.?theroux)\b', re.IGNORECASE)
MUSIC_EXT = re.compile(r'\.(mp3|flac|wav|ogg|m4a|alac|ape)\b', re.IGNORECASE)
MUSIC_K = re.compile(r'\b(discography|album|soundtrack|ost|single|ep|flac|lossless|320kbps|cdrip|vinyl|web-flac|remastered|anthology|greatest.?hits)\b', re.IGNORECASE)
ANIME_K = re.compile(r'\b(naruto|bleach|one.?piece|attack.?on.?titan|aot|demon.?slayer|kimetsu|dragon.?ball|fullmetal.?alchemist|evangelion|my.?hero.?academia|jujutsu.?kaisen|chainsaw.?man|spy.?x.?family|mob.?psycho|one.?punch.?man|frieren|dungeon.?meshi|solo.?leveling|kaiju.?no|dandadan|oshinoko|blue.?lock|haikyuu|baki|overlord|konosuba|mushoku.?tensei|slime.?datta|isekai|danmachi|horimiya|kaguya|bocchi|k-on|yuru.?camp|shokugeki|dr.?stone|fire.?force|black.?clover)\b', re.IGNORECASE)
ANIME_TAGS = re.compile(r'\[(subsplease|erai.?raws|horriblesubs|judas|dkb|beatrice|gabriel|dynamis|sofcj|sakurato|animetime|anime.?time|neoae|vcb-studio)\]', re.IGNORECASE)
WESTERN_ANIM = re.compile(r'\b(amphibia|owl.?house|gravity.?falls|star.?vs|steven.?universe|rick.?and.?morty|family.?guy|south.?park|futurama|american.?dad|bob.?s.?burgers|archer|bojack|big.?mouth|solar.?opposites|harley.?quinn|invincible|hazbin|helluva|clone.?wars|bad.?batch|young.?justice|batman.?animated|teen.?titans|adventure.?time|regular.?show|spongebob|avatar.?the.?last|simpsons|arcane)\b', re.IGNORECASE)
MOVIE_Q = re.compile(r'\b(1080p|720p|2160p|4k|uhd|blu.?ray|bluray|bdrip|web.?dl|webrip|x264|x265|hevc|hdrip|dvdrip|remux)\b', re.IGNORECASE)
TV_EP = re.compile(r'\bS\d{1,2}E\d{1,3}\b|\bSeason\s*\d+\b|\bComplete.?Series\b|\b\d{1,2}x\d{2}\b', re.IGNORECASE)
YEAR_P = re.compile(r'\b(19\d{2}|20[0-2]\d)\b')


def flatten_files(top_dirs):
    out = []
    for item in top_dirs:
        if isinstance(item, list):
            for x in item:
                if isinstance(x, str): out.append(x)
        elif isinstance(item, str):
            out.append(item)
    return out


def classify_strict(row):
    name = row.get("name", "")
    name_l = name.lower()
    fc = row.get("file_count", 0)
    files = flatten_files(row.get("top_dirs", []))
    files_str = " ".join(files).lower()
    all_text = name_l + " " + files_str

    if ADULT.search(all_text):
        return "Other"

    # Games
    if GAME_K.search(all_text) or GAME_EXT.search(files_str) or GAME_EXT.search(name_l):
        if not KNOWN_SW.search(name_l):
            return "Games"

    # Applications
    if APP_K.search(files_str) or (KNOWN_SW.search(name_l) and not MOVIE_Q.search(name_l)):
        return "Applications"
    if re.search(r'\b(windows\s*\d+|win\d+|ubuntu|debian|linux|macos)\b', name_l) and re.search(r'\.iso\b', files_str):
        return "Applications"

    # Documentaries
    if DOC_K.search(name_l) and not ADULT.search(all_text):
        return "Documentaries"

    # Music
    is_audio_name = bool(re.search(r'\.(mp3|flac|wav|ogg|m4a|alac|ape|aac|opus)\b', name_l))
    music_files = [f for f in files if MUSIC_EXT.search(f)]
    is_video = bool(re.search(r'\.(mkv|mp4|avi|ts|m4v|webm)$', name_l) or MOVIE_Q.search(name_l))
    
    if is_audio_name and not is_video:
        return "Music"
    if (len(music_files) >= 2 or (fc > 2 and MUSIC_EXT.search(name_l))) and not is_video:
        return "Music"
    if MUSIC_K.search(name_l) and (MUSIC_EXT.search(files_str) or fc > 1) and not is_video:
        return "Music"

    # Western Animation
    if WESTERN_ANIM.search(name_l) and not ANIME_K.search(name_l):
        return "Television"

    # Anime
    is_anime = bool(ANIME_K.search(name_l) or ANIME_TAGS.search(name_l))
    if is_anime:
        is_film = bool(re.search(r'\b(movie|film|gekijouban|the.?movie)\b', name_l))
        has_q = bool(MOVIE_Q.search(name_l))
        is_tv = bool(TV_EP.search(name))
        if is_film or (has_q and not is_tv and fc <= 3):
            return "Movies"
        return "Anime"

    # Television
    if TV_EP.search(name) or re.search(r'\b(HDTV|WEBRIP|AMZN|HULU|HBO|DISNEY\+|NETFLIX)\b', name, re.IGNORECASE) and fc >= 2:
        return "Television"

    # Movies
    if MOVIE_Q.search(name_l) and YEAR_P.search(name) and not TV_EP.search(name) and fc <= 4:
        return "Movies"

    return "Other"


def extract_balanced_benchmark(target_per_class=None, output_path="data/manual_eval_set_balanced_2000.jsonl"):
    caps = target_per_class or {
        "Games": 100,
        "Applications": 100,
        "Documentaries": 100,
        "Music": 150,
        "Anime": 200,
        "Movies": 300,
        "Television": 300,
        "Other": 400,
    }
    
    excluded = load_excluded()
    
    logger.info("Connecting to workspace-production...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect("workspace-production", username="core", password="rosrtdz@1995", timeout=10)
    
    # Query large sample from DB
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
    cmd = f'docker exec craw-db psql -U crawler -d craw -c "{query}"'
    
    logger.info("Streaming candidates from DB...")
    stdin, stdout, stderr = client.exec_command(cmd, bufsize=65536)
    
    collected = defaultdict(list)
    seen_in_run = set()
    total_scanned = 0
    
    for line in stdout:
        if not line.strip():
            continue
        total_scanned += 1
        try:
            row = json.loads(line)
        except Exception:
            continue
            
        ih = row.get("infohash", "").strip().lower()
        if not ih or ih in excluded or ih in seen_in_run:
            continue
            
        seen_in_run.add(ih)
        label = classify_strict(row)
        
        cap = caps.get(label, 200)
        if len(collected[label]) < cap:
            row["label_category"] = label
            collected[label].append(row)
            
        if all(len(collected[c]) >= caps[c] for c in caps):
            logger.info("All class quotas filled after scanning %d rows!", total_scanned)
            break
            
    client.close()
    
    all_bench = []
    for c, items in collected.items():
        all_bench.extend(items)
        
    logger.info("Total balanced benchmark samples: %d", len(all_bench))
    logger.info("Distribution: %s", {c: len(collected[c]) for c in sorted(caps.keys())})
    
    with open(output_path, "w", encoding="utf-8") as f:
        for r in all_bench:
            f.write(json.dumps(r) + "\n")
            
    logger.info("Saved balanced benchmark to %s", output_path)
    return len(all_bench)


if __name__ == "__main__":
    extract_balanced_benchmark()
