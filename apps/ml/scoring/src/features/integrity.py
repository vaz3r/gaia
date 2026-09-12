"""
Feature extraction for Torrent Integrity, Safety & Quality.
Extracts protocol geometry, structural file tree, and lexical signals.
"""
import math
import re
from typing import Dict, List, Any, Optional
from collections import Counter
from src.labels.silver_rules import (
    DECEPTIVE_EXT_PATTERN,
    NON_SOFTWARE_MEDIA_CATEGORIES,
    DANGEROUS_STANDALONE_EXTENSIONS,
    EXECUTABLE_EXTENSIONS,
    MAJOR_SOFTWARE_TITLES,
)

SPAM_KEYWORDS = {
    "crack", "keygen", "serial", "patch", "activation", "unlock",
    "free download", "full version", "registration code"
}

EXT_CATEGORIES = {
    "video": {".mp4", ".mkv", ".avi", ".wmv", ".mov", ".flv", ".webm", ".m4v", ".ts"},
    "audio": {".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a", ".wma", ".alac"},
    "executable": {".exe", ".msi", ".bat", ".cmd", ".scr", ".vbs", ".apk", ".dmg", ".pkg", ".deb", ".rpm"},
    "archive": {".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".iso"},
    "doc": {".pdf", ".epub", ".mobi", ".azw3", ".cbr", ".cbz", ".djvu", ".doc", ".docx"},
    "junk": {".nfo", ".txt", ".url", ".website", ".lnk", ".diz", ".ion"},
}


def shannon_entropy(text: str) -> float:
    """Calculate Shannon entropy of a string."""
    if not text:
        return 0.0
    length = len(text)
    counts = Counter(text)
    return -sum((c / length) * math.log2(c / length) for c in counts.values())


def extract_integrity_features(
    name: Optional[str] = None,
    total_size: int = 0,
    file_count: int = 0,
    piece_length: Optional[int] = None,
    files: Optional[List[Dict[str, Any]]] = None,
    category: Optional[str] = None,
) -> Dict[str, float]:
    """
    Vectorized feature extractor for integrity and malware classification.
    """
    total_size_safe = max(0, total_size)
    file_count_safe = max(1, file_count)
    piece_len_safe = max(1, piece_length or 262144)

    # 1. Category Context
    cat_lower = (category or "").strip().lower()
    cat_is_media = 1.0 if any(c in cat_lower for c in NON_SOFTWARE_MEDIA_CATEGORIES) else 0.0
    cat_is_software = 1.0 if any(c in cat_lower for c in ("application", "game")) else 0.0

    # 2. Geometry features
    log_size = math.log10(1.0 + total_size_safe)
    log_files = math.log10(1.0 + file_count_safe)
    log_piece = math.log10(1.0 + piece_len_safe)
    num_pieces = total_size_safe / piece_len_safe
    log_pieces = math.log10(1.0 + num_pieces)

    is_pow2 = 1.0 if (piece_len_safe & (piece_len_safe - 1) == 0) and piece_len_safe > 0 else 0.0

    # 3. File tree features
    cat_bytes = {k: 0 for k in EXT_CATEGORIES}
    depths = []
    zero_bytes = 0
    max_file_size = 0
    extensions = []
    has_deceptive_ext = 0.0
    has_non_ascii = 0.0
    has_rtlo = 0.0
    has_dangerous_script = 0.0
    executable_files_count = 0.0

    if files:
        for f in files:
            p_val = f.get("path") if isinstance(f, dict) else ""
            if isinstance(p_val, list):
                p = "/".join(str(seg) for seg in p_val)
            else:
                p = str(p_val or (f.get("name") if isinstance(f, dict) else ""))

            sz = max(0, int(f.get("length") or f.get("size") or 0)) if isinstance(f, dict) else 0
            if sz > max_file_size:
                max_file_size = sz
            if sz == 0:
                zero_bytes += 1

            # Path depth
            d = p.count("/") + p.count("\\")
            depths.append(d)

            # Extension
            ext = "." + p.rsplit(".", 1)[-1].lower() if "." in p else ""
            if ext:
                extensions.append(ext)
                for cat_name, ext_set in EXT_CATEGORIES.items():
                    if ext in ext_set:
                        cat_bytes[cat_name] += sz

            if ext in EXECUTABLE_EXTENSIONS:
                executable_files_count += 1.0

            if ext in DANGEROUS_STANDALONE_EXTENSIONS:
                has_dangerous_script = 1.0

            if DECEPTIVE_EXT_PATTERN.search(p):
                has_deceptive_ext = 1.0

            if any(ord(c) > 127 for c in p):
                has_non_ascii = 1.0

            if any(c in p for c in ("\u202E", "\u202D", "\u202C")):
                has_rtlo = 1.0

    # Share calculations
    denom = max(1, total_size_safe)
    video_share = cat_bytes["video"] / denom
    audio_share = cat_bytes["audio"] / denom
    executable_share = cat_bytes["executable"] / denom
    archive_share = cat_bytes["archive"] / denom
    doc_share = cat_bytes["doc"] / denom
    junk_share = cat_bytes["junk"] / denom
    primary_share = max_file_size / denom if files else 1.0

    max_depth = float(max(depths)) if depths else 0.0
    mean_depth = float(sum(depths) / len(depths)) if depths else 0.0
    zero_ratio = float(zero_bytes / file_count_safe)

    # Extension entropy
    ext_entropy = 0.0
    if extensions:
        ext_counts = Counter(extensions)
        tot_ext = len(extensions)
        ext_entropy = -sum((c / tot_ext) * math.log2(c / tot_ext) for c in ext_counts.values())

    # 4. Lexical features
    name_clean = name.strip() if name else ""
    title_len = float(len(name_clean))
    title_ent = shannon_entropy(name_clean)

    name_lower = name_clean.lower()
    spam_count = float(sum(1 for k in SPAM_KEYWORDS if k in name_lower))

    name_m = re.search(r"\s*\.([a-zA-Z0-9]+)\s*$", name_clean)
    name_ext = ("." + name_m.group(1).lower()) if name_m else ""
    if name_ext in EXECUTABLE_EXTENSIONS:
        executable_files_count += 1.0

    if name_ext in DANGEROUS_STANDALONE_EXTENSIONS:
        has_dangerous_script = 1.0

    if DECEPTIVE_EXT_PATTERN.search(name_clean):
        has_deceptive_ext = 1.0

    if any(c in name_clean for c in ("\u202E", "\u202D", "\u202C")):
        has_rtlo = 1.0

    # 5. Cross-Category Security & Interaction Features
    media_exe_mismatch = 1.0 if (cat_is_media == 1.0 and (executable_share > 0.0 or executable_files_count > 0)) else 0.0
    media_standalone_exe = 1.0 if (cat_is_media == 1.0 and file_count_safe <= 2 and (executable_share > 0.4 or name_ext in EXECUTABLE_EXTENSIONS)) else 0.0
    
    software_implausible_size = 0.0
    if cat_is_software == 1.0 and 0 < total_size_safe < 15 * 1024 * 1024:
        if MAJOR_SOFTWARE_TITLES.search(name_clean):
            software_implausible_size = 1.0

    return {
        "log_total_size": log_size,
        "log_file_count": log_files,
        "log_num_pieces": log_pieces,
        "piece_length_is_pow2": is_pow2,
        "max_path_depth": max_depth,
        "mean_path_depth": mean_depth,
        "video_share": video_share,
        "audio_share": audio_share,
        "executable_share": executable_share,
        "archive_share": archive_share,
        "doc_share": doc_share,
        "junk_share": junk_share,
        "primary_ext_share": primary_share,
        "extension_entropy": ext_entropy,
        "zero_byte_file_ratio": zero_ratio,
        "title_length": title_len,
        "title_entropy": title_ent,
        "spam_keyword_count": spam_count,
        "path_has_non_ascii": has_non_ascii,
        "deceptive_double_extension": has_deceptive_ext,
        "category_is_media": cat_is_media,
        "category_is_software": cat_is_software,
        "media_executable_mismatch": media_exe_mismatch,
        "media_standalone_executable": media_standalone_exe,
        "has_rtlo_spoofing": has_rtlo,
        "has_dangerous_script": has_dangerous_script,
        "software_implausible_size": software_implausible_size,
        "executable_file_count": executable_files_count,
    }
