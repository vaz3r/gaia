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

    video = 0
    audio = 0
    subs = 0
    archives = 0
    installs = 0
    docs = 0
    images = 0

    ext_counts = Counter()
    for f in files:
        if isinstance(f, str):
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

    # Documentaries (takes precedence over general TV/Movies if clearly factual)
    is_doc = bool(re.search(
        r"\b(BBC|PBS|NOVA|National\s*Geographic|Nat\s*Geo|Discovery(\s*Channel)?|Frontline|Horizon|NHK|CuriosityStream|Panorama|Disneynature|Documentary|Docuseries|David\s*Attenborough|History\s*Channel|DW\s*Documentary)\b",
        name, re.IGNORECASE
    ))

    # Anime / Fansub release patterns
    is_fansub = bool(re.search(
        r"\[(Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|Nekomoe|Tenshi)\]",
        name, re.IGNORECASE
    ))
    is_anime_station = bool(re.search(r"\b(AT-X|Tokyo\s*MX|BS11|MBS|TBS|TV\s*Tokyo|KBS|Animax|Crunchyroll|Funimation|HIDIVE)\b", name, re.IGNORECASE))
    is_anime_ep = bool(re.search(r"\s-\s\d{1,4}(\s*v\d)?\s*(\(|\[|\.mkv|\.mp4)", name)) or bool(re.search(r"\b(Batch|OVA|ONA|Special|SP\d{1,2})\b", name, re.IGNORECASE) and (is_fansub or is_anime_station))
    
    # Games release patterns
    is_game_scene = bool(re.search(
        r"\b(FitGirl|CODEX|PLAZA|DODI|SKIDROW|RUNE|FLT|EMPRESS|TENOKE|Razor1911|PROPHET|GOG|ElAmigos|KaOs|TinyISO|TiNYiSO|CPY|HOODLUM|RELOADED|DARKSiDERS|TiNYiSO|Goldberg|SteamRip|Steam-Rip)\b",
        name, re.IGNORECASE
    ))
    is_console_rom = bool(re.search(r"\b(NSP|XCI|NSZ|CIA|3DS|VPK|WAD|WBFS|CSO|GCM|NDS|GBA|PS3|PS4|PKG|Switch\s*ROM|ROM\s*Pack|v1\.\d+\.\d+(\s*DLC|\s*Update)?)\b", name, re.IGNORECASE)) or any(
        f.lower().endswith(('.nsp', '.xci', '.nsz', '.cia', '.3ds', '.vpk', '.wbfs', '.cso', '.nds', '.gba')) for f in files
    )
    
    # Applications patterns
    is_app_vendor = bool(re.search(
        r"\b(Adobe|Autodesk|JetBrains|Microsoft\s*Office|Windows\s*(10|11|Server)|macOS|WinPE|AutoCAD|SolidWorks|Ableton|FL\s*Studio|Cubase|CorelDRAW|MATLAB|VMware|IntelliJ|Photoshop|Illustrator|Premiere|Acrobat|Kaspersky|Bitdefender|CCleaner|Acronis|EaseUS|Tenorshare|Driver\s*Booster|Office\s*20\d{2})\b",
        name, re.IGNORECASE
    ))
    is_app_payload = bool(re.search(r"\b(v\d+\.\d+(\.\d+)?|x64|x86|x32|Multilingual|Portable|Crack|Keygen|Patch|Activator|Plugin|VST|VST3|AAX|Win64|macOS|Android|APK|iOS|IPA)\b", name, re.IGNORECASE))

    # Books / E-books / Comics
    is_ebook = any(f.lower().endswith(('.epub', '.mobi', '.azw3', '.pdf', '.cbr', '.cbz')) for f in files) or bool(re.search(r"\b(eBook|e-book|Audiobook|Comic|Manga|Magazine|EPUB|MOBI)\b", name, re.IGNORECASE))
    
    # Courses / Educational
    is_course = bool(re.search(r"\b(Udemy|Coursera|Pluralsight|Tutorial|Training|Masterclass|LinkedIn\s*Learning|Frontend\s*Masters|Packt|O'?Reilly)\b", name, re.IGNORECASE))

    # JAV / Adult
    is_adult = bool(re.search(r"\b([A-Z]{3,5}-\d{3,5}|OnlyFans|Brazzers|NaughtyAmerica|BangBros|TeamSkeet|Adult|JAV|Porn|Hentai|18\+|XXX|Uncensored|FC2-PPV-\d+)\b", name, re.IGNORECASE))

    if is_doc:
        parts.append("Type: Documentary")
    elif is_fansub or (is_anime_ep and (is_anime_station or not re.search(r"\b(S\d{1,2}E\d{1,3}|Season\s+\d)\b", name, re.IGNORECASE))):
        parts.append("Type: Anime")
    elif is_game_scene or is_console_rom:
        parts.append("Type: Game release")
    elif is_app_vendor or (is_app_payload and installs and not video and not is_game_scene):
        parts.append("Type: Application software")
    elif is_adult:
        parts.append("Type: Adult media")
    elif is_course:
        parts.append("Type: Course tutorial")
    elif is_ebook and not video:
        parts.append("Type: Book publication")
    elif re.search(r"S\d{1,2}E\d{1,3}", name, re.IGNORECASE):
        if is_anime_station:
            parts.append("Type: Anime episode")
        else:
            parts.append("Type: TV episode")
    elif re.search(r"Season\s+\d|S\d{1,2}\s", name, re.IGNORECASE):
        parts.append("Type: TV season")
    elif re.search(r"\b(19|20)\d{2}\b", name) and re.search(r"\b(1080p|720p|4K|BluRay|WEBRip|BDRip|HDRip|WEB-DL|x264|x265|HEVC)\b", name, re.IGNORECASE):
        parts.append("Type: Movie release")
    elif re.search(r"\bmkv$|\.avi$|\.mp4$", name, re.IGNORECASE):
        parts.append("Type: Video file")
        
    if re.search(r"\b(anime|batch|fansub)\b", name, re.IGNORECASE) and "Type: Anime" not in parts:
        parts.append("Type: Anime")
        
    # Audio formatting (only if not an obvious video container)
    is_video_container = bool(re.search(r"\.(mkv|mp4|avi|ts|m4v|webm)$", name, re.IGNORECASE))
    if not is_video_container:
        if re.search(r"\bFLAC\b", name, re.IGNORECASE):
            parts.append("Format: FLAC audio")
        elif re.search(r"\b(MP3|320kbps)\b", name, re.IGNORECASE):
            parts.append("Format: MP3 audio")
        elif audio and not video and not installs:
            parts.append("Type: Music album")
            
    if re.search(r"\b(dub|dubbed|VO|VOSTFR|VOST)\b", name, re.IGNORECASE):
        parts.append("Audio: dubbed")
    if re.search(r"\b(sub|subbed|subs|softsub|MultiSub)\b", name, re.IGNORECASE):
        parts.append("Subtitled")
    if re.search(r"\b(crack|keygen|patch|serial)\b", name, re.IGNORECASE):
        parts.append("DRM: cracked")
    if re.search(r"\b(repack|fitgirl|corepack|dodi)\b", name, re.IGNORECASE):
        parts.append("Format: repack")

    return "; ".join(parts)


def _parse_files(torrent: dict | TorrentInput) -> tuple[str, int, int, list[str], list[dict]]:
    """Parse common fields from torrent dict or TorrentInput."""
    if isinstance(torrent, TorrentInput):
        name = torrent.name
        file_count = torrent.file_count
        total_size = torrent.total_size_bytes
        raw_files = torrent.files
    else:
        name = str(torrent.get("name", ""))
        file_count = torrent.get("file_count", 0)
        total_size = torrent.get("total_size", torrent.get("total_size_bytes", 0))
        raw_files = torrent.get("top_dirs", torrent.get("files", [])) or []

    files = _flatten_files(raw_files)
    return name, file_count, total_size, files, raw_files


def _flatten_files(raw_files) -> list[str]:
    """Flatten various file list formats to simple string paths."""
    if not raw_files or not isinstance(raw_files, list):
        return []
    if isinstance(raw_files[0], dict):
        out = []
        for f in raw_files:
            path = f.get("path", [])
            if isinstance(path, list):
                out.append("/".join(str(p) for p in path))
            else:
                out.append(str(path))
        return out
    if isinstance(raw_files[0], list):
        return ["/".join(str(p) for p in f) for f in raw_files]
    return [str(f) for f in raw_files if isinstance(f, str)]


def _extract_raw_metadata(files: list[str]) -> tuple[list[str], list[str], list[str]]:
    """Extract extensions, top folders, and file names from flat file paths."""
    extensions = []
    top_folders = set()
    file_names = []
    for f in files:
        if not isinstance(f, str):
            continue
        # Extension
        dot = f.rfind(".")
        if dot >= 0:
            ext = f[dot:].lower()
            if ext not in extensions:
                extensions.append(ext)
        # Top folder (first path component)
        slash = f.find("/")
        if slash > 0:
            top_folders.add(f[:slash])
        # File name (last component)
        fname = f.rsplit("/", 1)[-1] if "/" in f else f
        file_names.append(fname)
    return extensions[:10], sorted(top_folders)[:10], file_names


def _build_raw_text(
    name: str, file_count: int, total_size: int,
    extensions: list[str], top_folders: list[str], file_names: list[str],
    max_name_chars: int,
) -> str:
    """Build raw metadata text representation (no regex features)."""
    name = name[:max_name_chars]
    lines = [f"Name: {name}", f"Files: {file_count}  Size: {total_size}"]
    if extensions:
        lines.append(f"Extensions: {', '.join(extensions)}")
    if top_folders:
        lines.append(f"Top folders: {', '.join(top_folders)}")
    if file_names:
        lines.append(f"File names: {', '.join(file_names[:10])}")
    return "\n".join(lines)


def build_input_text(torrent: dict | TorrentInput, config: dict | None = None) -> str:
    """Build plain-text representation for embedding.

    This function MUST be used by both training and inference
    to ensure identical text format. Do not duplicate this logic.

    Config options:
        text_builder.mode: "full" (default) or "raw"
            - "full": includes regex-based feature detection (fansub tags, scene groups, etc.)
            - "raw":  raw metadata only (name, file_count, size, extensions, folders, file names)
        text_builder.max_files: max files to include (default 10)
        text_builder.max_file_chars: max chars per file path (default 100)
        text_builder.max_name_chars: max chars for name (default 300)
    """
    cfg = (config or {}).get("text_builder", {})
    mode = cfg.get("mode", "full")
    max_files = cfg.get("max_files", 10)
    max_file_chars = cfg.get("max_file_chars", 100)
    max_name_chars = cfg.get("max_name_chars", 300)

    name, file_count, total_size, files, _ = _parse_files(torrent)

    if mode == "raw":
        # Handle pre-extracted metadata (from MCP server) or extract from file paths
        if isinstance(torrent, dict):
            extensions = torrent.get("extensions", [])
            top_folders = torrent.get("top_folders", [])
            largest_files = torrent.get("largest_files", [])
        else:
            extensions, top_folders, _ = _extract_raw_metadata(files)
            largest_files = []

        name = name[:max_name_chars]
        lines = [f"Name: {name}", f"Files: {file_count}  Size: {total_size}"]
        if extensions:
            lines.append(f"Extensions: {', '.join(str(e) for e in extensions)}")
        if top_folders:
            lines.append(f"Top folders: {', '.join(str(f) for f in top_folders)}")
        if largest_files:
            if isinstance(largest_files[0], dict):
                file_strs = [f"{f.get('name', '?')} ({f.get('size', 0)})" for f in largest_files]
            else:
                file_strs = [str(f) for f in largest_files]
            lines.append(f"Largest files: {', '.join(file_strs)}")
        return "\n".join(lines)

    # Full mode: includes regex feature detection
    name = name[:max_name_chars]
    top_dirs = files[:max_files]
    dirs_str = ", ".join(d[:max_file_chars] for d in top_dirs)
    features = _detect_features(name, files)

    lines = [f"Name: {name}", f"Files: {file_count}  Size: {total_size}"]
    if features:
        lines.append(features)
    lines.append(f"Top dirs: {dirs_str}")
    return "\n".join(lines)
