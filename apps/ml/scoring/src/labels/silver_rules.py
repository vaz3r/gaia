"""
High-precision deterministic silver rules and critical invariant checks.
"""
import re
import unicodedata
from typing import Dict, List, Any, Tuple, Optional
from src.common.types import ReasonCode

# Deceptive executable extensions disguised as media/documents
DECEPTIVE_EXT_PATTERN = re.compile(
    r"\.(mp4|mkv|avi|wmv|mov|flv|webm|mp3|flac|wav|aac|zip|rar|7z|tar|iso|pdf|epub)\.(exe|scr|bat|cmd|vbs|vbe|js|jse|wsf|wsh|ps1|com)$",
    re.IGNORECASE,
)

# Password traps & unlocker scams
PASSWORD_TRAP_FILENAMES = {
    "password.txt", "passwords.txt", "password_unlock.txt",
    "unlock_key.txt", "readme_password.txt", "how_to_open.txt",
    "key_generator.url", "password_link.url", "unlock.url"
}

# Categories where <= 1KB payload is impossible for legitimate content
MEDIA_AND_SOFTWARE_CATEGORIES = {
    "adult", "television", "movies", "games", "applications", "music", "anime"
}

# Unicode homoglyphs: Cyrillic letters that look identical to Latin letters
CYRILLIC_LOOKALIKES = set("асеорхуАВЕКМНОРСТХ")


def check_critical_sha1_mismatch(last_error: Optional[str] = None) -> bool:
    """Flag critical SHA1 metadata hash mismatch."""
    if not last_error:
        return False
    return "sha1_mismatch" in last_error.lower()


def check_empty_payload_fake(total_size: int, category: Optional[str] = None) -> bool:
    """Flag empty/near-empty payloads claiming to be media or software."""
    if total_size <= 0:
        return True
    if total_size <= 1024:
        if not category:
            return True
        cat_lower = category.strip().lower()
        if any(c in cat_lower for c in MEDIA_AND_SOFTWARE_CATEGORIES):
            return True
    return False


def _get_path_str(f: Any) -> str:
    if not isinstance(f, dict):
        return str(f)
    p = f.get("path") if "path" in f else f.get("name", "")
    if isinstance(p, list):
        return "/".join(str(seg) for seg in p)
    return str(p or "")


def check_deceptive_double_extension(files: Optional[List[Dict[str, Any]]] = None, name: Optional[str] = None) -> Tuple[bool, List[str]]:
    """Flag filenames like movie.mp4.exe or invoice.pdf.scr."""
    flagged = []
    if name and DECEPTIVE_EXT_PATTERN.search(name):
        flagged.append(name)

    if files:
        for f in files:
            p = _get_path_str(f)
            if DECEPTIVE_EXT_PATTERN.search(p):
                flagged.append(p)

    return len(flagged) > 0, flagged[:5]


def check_password_trap(files: Optional[List[Dict[str, Any]]] = None) -> Tuple[bool, List[str]]:
    """Flag presence of password lock/unlock instructions."""
    if not files:
        return False, []
    traps = []
    for f in files:
        p = _get_path_str(f).lower()
        basename = p.split("/")[-1]
        if basename in PASSWORD_TRAP_FILENAMES or "password" in basename and (basename.endswith(".txt") or basename.endswith(".url")):
            traps.append(basename)
    return len(traps) > 0, traps[:3]


def check_homoglyph_spoofing(text: Optional[str] = None) -> bool:
    """Detect suspicious Cyrillic-Latin homoglyph mixing within single words."""
    if not text:
        return False
    words = re.findall(r"[A-Za-z\u0400-\u04FF]+", text)
    for word in words:
        has_latin = bool(re.search(r"[A-Za-z]", word))
        has_cyrillic = bool(re.search(r"[\u0400-\u04FF]", word))
        if has_latin and has_cyrillic:
            return True
    return False


def evaluate_silver_invariants(
    name: str,
    total_size: int,
    category: Optional[str] = None,
    files: Optional[List[Dict[str, Any]]] = None,
    last_error: Optional[str] = None,
) -> Tuple[List[ReasonCode], Dict[str, Any]]:
    """
    Evaluates all deterministic invariants.
    Returns list of triggered ReasonCodes and diagnostic details.
    """
    reasons = []
    details = {}

    if check_critical_sha1_mismatch(last_error):
        reasons.append(ReasonCode.CRITICAL_SHA1_MISMATCH)
        details["sha1_mismatch"] = True

    if check_empty_payload_fake(total_size, category):
        reasons.append(ReasonCode.EMPTY_PAYLOAD_FAKE)
        details["empty_payload"] = {"total_size": total_size, "category": category}

    has_double_ext, flagged_exts = check_deceptive_double_extension(files, name)
    if has_double_ext:
        reasons.append(ReasonCode.DECEPTIVE_DOUBLE_EXTENSION)
        details["deceptive_extensions"] = flagged_exts

    has_pw, pw_files = check_password_trap(files)
    if has_pw:
        reasons.append(ReasonCode.PASSWORD_TRAP_SUSPECTED)
        details["password_traps"] = pw_files

    if check_homoglyph_spoofing(name):
        reasons.append(ReasonCode.HOMOGLYPH_PATH_SPOOFING)
        details["homoglyph"] = True

    return reasons, details
