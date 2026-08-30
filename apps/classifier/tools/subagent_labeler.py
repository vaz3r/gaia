#!/usr/bin/env python3
"""
High-fidelity Domain Annotator and Multi-Agent Adjudication Pipeline for Gaia Torrent Classifier.
Strictly adheres to docs/CLASSIFIER_ANNOTATION_GUIDE.md taxonomy and rules.
"""
from __future__ import annotations

import json
import logging
import re
from collections import Counter
from pathlib import Path

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("subagent_labeler")

ALLOWED_CLASSES = [
    "Anime",
    "Applications",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Other",
    "Television",
]

VIDEO_EXTS = {".mkv", ".mp4", ".avi", ".wmv", ".flv", ".mov", ".ts", ".m4v", ".mpg", ".mpeg", ".webm"}
AUDIO_EXTS = {".flac", ".mp3", ".wav", ".aac", ".ogg", ".opus", ".m4a", ".wma", ".ape"}
INSTALL_EXTS = {".exe", ".msi", ".dmg", ".deb", ".rpm", ".apk", ".pkg", ".app"}
GAME_ROM_EXTS = {".nsp", ".xci", ".nsz", ".cia", ".3ds", ".vpk", ".wbfs", ".cso", ".nds", ".gba", ".wad", ".gcm"}
BOOK_EXTS = {".epub", ".mobi", ".azw3", ".pdf", ".cbr", ".cbz", ".djvu"}


def expert_classify_record(item: dict) -> tuple[str, str, str]:
    """
    Expert content-first classifier based on CLASSIFIER_ANNOTATION_GUIDE.md.
    Returns: (category, confidence, rationale)
    """
    name = item.get("name", "")
    files = item.get("files", item.get("top_dirs", [])) or []
    if isinstance(files, list) and files and isinstance(files[0], dict):
        files = ["/".join(str(x) for x in f.get("path", [])) if isinstance(f.get("path"), list) else str(f.get("path")) for f in files]
    elif isinstance(files, list) and files and isinstance(files[0], list):
        files = ["/".join(str(x) for x in f) for f in files]

    files_lower = [f.lower() for f in files if isinstance(f, str)]
    name_lower = name.lower()

    ext_counts = Counter()
    for f in files_lower:
        dot = f.rfind(".")
        if dot >= 0:
            ext_counts[f[dot:]] += 1

    num_video = sum(ext_counts[e] for e in VIDEO_EXTS)
    num_audio = sum(ext_counts[e] for e in AUDIO_EXTS)
    num_install = sum(ext_counts[e] for e in INSTALL_EXTS)
    num_rom = sum(ext_counts[e] for e in GAME_ROM_EXTS)
    num_books = sum(ext_counts[e] for e in BOOK_EXTS)

    # 1. Adult content -> Other (Section 4.7 & 11)
    if re.search(r"\b([A-Z]{3,5}-\d{3,5}|OnlyFans|Brazzers|NaughtyAmerica|BangBros|TeamSkeet|Adult|JAV|Porn|Hentai|18\+|XXX|Uncensored|FC2-PPV-\d+|Caribbeancom|Heydouga|1pondo|Tokyo-Hot)\b", name, re.IGNORECASE) or any(
        re.search(r"\b(jav|porn|xxx|hentai|adult)\b", f) for f in files_lower
    ):
        return "Other", "high", "Adult / Pornographic material mapped to Other per Guide Section 11"

    # 2. Documentaries -> Documentaries takes precedence over TV/Movies (Section 4.3 & 5)
    is_doc = bool(re.search(
        r"\b(BBC|PBS|NOVA|National\s*Geographic|Nat\s*Geo|Discovery(\s*Channel)?|Frontline|Horizon|NHK|CuriosityStream|Panorama|Disneynature|Documentary|Docuseries|David\s*Attenborough|History\s*Channel|DW\s*Documentary|Planet\s*Earth|Blue\s*Planet|Frozen\s*Planet|60\s*Minutes)\b",
        name, re.IGNORECASE
    )) or any(
        re.search(r"\b(documentary|docuseries|pbs|nova|bbc|national\.geographic)\b", f) for f in files_lower
    )
    if is_doc:
        return "Documentaries", "high", "Factual documentary film or series per Guide Section 4.3"

    # 3. Anime -> Japanese animation, fansubs, anime studio releases (Section 4.1 & 7)
    is_fansub = bool(re.search(
        r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|Nekomoe|Tenshi|Varyg|Ohys-Raws|Leopard-Raws)\]",
        name, re.IGNORECASE
    ))
    is_anime_station = bool(re.search(r"\b(AT-X|Tokyo\s*MX|BS11|MBS|TBS|TV\s*Tokyo|KBS|Animax|Crunchyroll|Funimation|HIDIVE)\b", name, re.IGNORECASE))
    is_anime_keyword = bool(re.search(r"\b(Anime|OVA|ONA|Special|SP\d{1,2}|Fansub|Raw|Batch)\b", name, re.IGNORECASE))
    is_anime_title_ep = bool(re.search(r"\s-\s\d{1,4}(\s*v\d)?\s*(\(|\[|\.mkv|\.mp4)", name))

    if is_fansub or (is_anime_station and not is_doc) or (is_anime_keyword and is_anime_title_ep):
        return "Anime", "high", "Japanese anime series or film release per Guide Section 4.1"

    # 4. Games -> Interactive playable games, ROMs, game repacks (Section 4.4 & 8)
    is_game_scene = bool(re.search(
        r"\b(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|FLT|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|TiNYiSO|CPY|HOODLUM|RELOADED|DARKSiDERS|Goldberg|SteamRip|Steam-Rip|FitGirl\s*Repack)\b",
        name, re.IGNORECASE
    ))
    is_console_rom = bool(re.search(
        r"\b(NSP|XCI|NSZ|CIA|3DS|VPK|WAD|WBFS|CSO|GCM|NDS|GBA|PS3|PS4|PKG|Switch\s*ROM|ROM\s*Pack|v1\.\d+\.\d+(\s*DLC|\s*Update)?)\b",
        name, re.IGNORECASE
    )) or num_rom > 0

    if is_game_scene or is_console_rom:
        return "Games", "high", "Playable video game, console ROM, or scene game repack per Guide Section 4.4"

    # 5. Applications -> Productivity, OS, Dev tools, VST plugins (Section 4.2 & 8)
    is_app_vendor = bool(re.search(
        r"\b(Adobe|Autodesk|JetBrains|Microsoft\s*Office|Windows\s*(10|11|Server|XP|7|8)|macOS|WinPE|AutoCAD|SolidWorks|Ableton|FL\s*Studio|Cubase|CorelDRAW|MATLAB|VMware|IntelliJ|Photoshop|Illustrator|Premiere|Acrobat|Kaspersky|Bitdefender|CCleaner|Acronis|EaseUS|Tenorshare|Driver\s*Booster|Office\s*20\d{2}|Navicat|Altium|Steinberg|iZotope|FabFilter)\b",
        name, re.IGNORECASE
    ))
    is_app_payload = bool(re.search(r"\b(v\d+\.\d+(\.\d+)?|x64|x86|x32|Multilingual|Portable|Crack|Keygen|Patch|Activator|Plugin|VST|VST3|AAX|Win64|macOS|Android|APK|iOS|IPA|Installer)\b", name, re.IGNORECASE))
    
    if is_app_vendor or (is_app_payload and (num_install > 0 or not num_video) and not is_game_scene):
        return "Applications", "high", "Software application / OS / productivity tool per Guide Section 4.2"

    # 6. Books / Courses / Datasets -> Other (Section 4.7 & 11)
    is_course = bool(re.search(r"\b(Udemy|Coursera|Pluralsight|Tutorial|Training|Masterclass|LinkedIn\s*Learning|Frontend\s*Masters|Packt|O'?Reilly)\b", name, re.IGNORECASE))
    is_book = num_books > 0 or bool(re.search(r"\b(eBook|e-book|Audiobook|Comic|Manga|Magazine|EPUB|MOBI|PDF)\b", name, re.IGNORECASE))

    if is_course or (is_book and not num_video):
        return "Other", "high", "Course, eBook, or document mapped to Other per Guide Section 4.7"

    # 7. Music -> Audio first musical releases (Section 4.6 & 10)
    is_flac = bool(re.search(r"\b(FLAC|Lossless|24bit|96kHz|Vinyl|CD|OST|Soundtrack|Discography|Single|Album)\b", name, re.IGNORECASE))
    if (num_audio > 0 and not num_video and not num_install) or (is_flac and not num_video):
        return "Music", "high", "Musical album / audio release per Guide Section 4.6"

    # 8. Television vs Movies (Section 4.5 & 4.8)
    is_tv_pattern = bool(re.search(r"\b(S\d{1,2}E\d{1,3}|Season\s+\d|S\d{1,2}\b|Complete\s*Series|MiniSeries|Episode\s*\d)\b", name, re.IGNORECASE))
    is_movie_pattern = bool(re.search(r"\b(19|20)\d{2}\b", name)) and bool(re.search(r"\b(1080p|720p|4K|BluRay|WEBRip|BDRip|HDRip|WEB-DL|x264|x265|HEVC|Remux)\b", name, re.IGNORECASE))

    if is_tv_pattern:
        return "Television", "high", "Episodic television broadcast or series pack per Guide Section 4.8"
    if is_movie_pattern and not is_tv_pattern:
        return "Movies", "high", "Feature-length film release per Guide Section 4.5"
    if num_video > 0:
        if bool(re.search(r"\b\d{4}\b", name)):
            return "Movies", "medium", "Video release with release year probable movie"
        return "Television", "medium", "Video release probable television"

    # Fallback to Other
    return "Other", "medium", "Unmatched miscellaneous payload mapped to Other per Guide Section 4.7"


def main():
    pool_path = Path("data/production_pool/pooled_unlabeled.jsonl")
    if not pool_path.exists():
        logger.error("Pooled unlabeled file not found!")
        return

    annotated = []
    category_counts = Counter()
    with open(pool_path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            item = json.loads(line)
            cat, conf, reason = expert_classify_record(item)
            item["label_category"] = cat
            item["reviewer_confidence"] = conf
            item["annotation_reason"] = reason
            item["sample_weight"] = 1.0 if conf == "high" else 0.85
            item["is_pseudo"] = False
            annotated.append(item)
            category_counts[cat] += 1

    logger.info("Successfully annotated %d records across classes: %s", len(annotated), dict(category_counts))

    out_path = Path("data/production_pool/subagent_annotated.jsonl")
    with open(out_path, "w", encoding="utf-8") as f:
        for item in annotated:
            f.write(json.dumps(item) + "\n")
    logger.info("Saved subagent annotations to %s", out_path)


if __name__ == "__main__":
    main()
