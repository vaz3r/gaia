from __future__ import annotations

import re
from collections import Counter
from .types import TorrentInput


VIDEO_EXTS = {".mkv", ".mp4", ".avi", ".wmv", ".flv", ".mov", ".ts", ".m4v", ".mpg", ".mpeg", ".webm"}
AUDIO_EXTS = {".flac", ".mp3", ".wav", ".aac", ".ogg", ".opus", ".m4a", ".wma"}
SUBTITLE_EXTS = {".srt", ".ass", ".ssa", ".sub", ".idx", ".sup"}
IMAGE_EXTS = {".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp"}
ARCHIVE_EXTS = {".zip", ".rar", ".7z", ".tar", ".gz", ".bz2"}
INSTALL_EXTS = {".exe", ".msi", ".dmg", ".deb", ".rpm", ".apk"}
DOC_EXTS = {".pdf", ".epub", ".mobi", ".txt", ".doc", ".docx"}


def _detect_features(name: str, files: list[str]) -> str:
    parts = []

    ext_counts = Counter()
    for f in files:
        dot = f.rfind(".")
        if dot >= 0:
            ext_counts[f[dot:].lower()] += 1
    if ext_counts:
        video = sum(ext_counts.get(e, 0) for e in VIDEO_EXTS)
        audio = sum(ext_counts.get(e, 0) for e in AUDIO_EXTS)
        subs = sum(ext_counts.get(e, 0) for e in SUBTITLE_EXTS)
        archives = sum(ext_counts.get(e, 0) for e in ARCHIVE_EXTS)
        installs = sum(ext_counts.get(e, 0) for e in INSTALL_EXTS)
        docs = sum(ext_counts.get(e, 0) for e in DOC_EXTS)
        images = sum(ext_counts.get(e, 0) for e in IMAGE_EXTS)
        tags = []
        if video: tags.append(f"{video} video")
        if audio: tags.append(f"{audio} audio")
        if subs: tags.append(f"{subs} subtitle")
        if archives: tags.append(f"{archives} archive")
        if installs: tags.append(f"{installs} installer")
        if docs: tags.append(f"{docs} document")
        if images: tags.append(f"{images} image")
        if tags:
            parts.append("Media: " + ", ".join(tags))

    # Anime / Fansub release patterns
    is_fansub = bool(re.search(r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa)\]", name, re.IGNORECASE))
    is_anime_ep = bool(re.search(r"\s-\s\d{1,4}(\s*v\d)?\s*\[", name))
    
    if re.search(r"S\d{1,2}E\d{1,3}", name, re.IGNORECASE):
        if is_fansub or re.search(r"\b(ANi|Baha|AT-X|Tokyo\s*MX|BS11)\b", name, re.IGNORECASE):
            parts.append("Type: Anime episode")
        else:
            parts.append("Type: TV episode")
    elif is_fansub or (is_anime_ep and re.search(r"\b(1080p|720p|480p|CR|B-Global|ADN|Baha)\b", name, re.IGNORECASE)):
        parts.append("Type: Anime episode")
    elif re.search(r"Season\s+\d|S\d{1,2}\s", name, re.IGNORECASE):
        parts.append("Type: TV season")
    elif re.search(r"\b(19|20)\d{2}\b", name) and re.search(r"\b(1080p|720p|4K|BluRay|WEBRip|BDRip|HDRip|WEB-DL|x264|x265|HEVC)\b", name, re.IGNORECASE):
        parts.append("Type: Movie release")
    elif re.search(r"\bmkv$|\.avi$|\.mp4$", name, re.IGNORECASE):
        parts.append("Type: Video file")
        
    if re.search(r"\b(anime|batch|fansub)\b", name, re.IGNORECASE):
        parts.append("Type: Anime")
        
    # Audio formatting (only if not an obvious video container)
    is_video_container = bool(re.search(r"\.(mkv|mp4|avi|ts|m4v|webm)$", name, re.IGNORECASE))
    if not is_video_container:
        if re.search(r"\bFLAC\b", name, re.IGNORECASE):
            parts.append("Format: FLAC audio")
        elif re.search(r"\b(MP3|320kbps)\b", name, re.IGNORECASE):
            parts.append("Format: MP3 audio")
            
    if re.search(r"\b(dub|dubbed|VO|VOSTFR|VOST)\b", name, re.IGNORECASE):
        parts.append("Audio: dubbed")
    if re.search(r"\b(sub|subbed|subs|softsub|MultiSub)\b", name, re.IGNORECASE):
        parts.append("Subtitled")
    if re.search(r"\b(crack|keygen|patch|serial)\b", name, re.IGNORECASE):
        parts.append("DRM: cracked")
    if re.search(r"\b(repack|fitgirl|corepack|dodi)\b", name, re.IGNORECASE):
        parts.append("Format: repack")

    return "; ".join(parts)


def build_input_text(torrent: dict | TorrentInput, config: dict | None = None) -> str:
    """Build plain-text representation for embedding.

    This function MUST be used by both training and inference
    to ensure identical text format. Do not duplicate this logic.
    """
    cfg = (config or {}).get("text_builder", {})
    max_files = cfg.get("max_files", 10)
    max_file_chars = cfg.get("max_file_chars", 100)
    max_name_chars = cfg.get("max_name_chars", 300)

    if isinstance(torrent, TorrentInput):
        name = torrent.name
        file_count = torrent.file_count
        total_size = torrent.total_size_bytes
        files = torrent.files
    else:
        name = str(torrent.get("name", ""))
        file_count = torrent.get("file_count", 0)
        total_size = torrent.get("total_size", torrent.get("total_size_bytes", 0))
        files = torrent.get("top_dirs", torrent.get("files", [])) or []

    if isinstance(files, list) and len(files) > 0:
        if isinstance(files[0], dict):
            # Flatten dict structure to paths
            str_files = []
            for f in files:
                path = f.get('path', [])
                if isinstance(path, list):
                    str_files.append("/".join(str(p) for p in path))
                else:
                    str_files.append(str(path))
            files = str_files
        elif isinstance(files[0], list):
            # Flatten list of lists (PostgreSQL jsonb array format)
            files = ["/".join(str(p) for p in f) for f in files]

    name = name[:max_name_chars]
    top_dirs = files[:max_files]
    dirs_str = ", ".join(d[:max_file_chars] for d in top_dirs)
    features = _detect_features(name, files)

    lines = [f"Name: {name}", f"Files: {file_count}  Size: {total_size}"]
    if features:
        lines.append(features)
    lines.append(f"Top dirs: {dirs_str}")
    return "\n".join(lines)
