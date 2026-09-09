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


def check_password_trap(files: Optional[List[Dict[str, Any]]] = None) -> Tuple[bool, bool, List[str]]:
    """
    Flag presence of password lock/unlock instructions.
    Returns: (has_external_trap, has_local_note, trap_files)
    """
    if not files:
        return False, False, []
    
    external_traps = []
    local_notes = []

    for f in files:
        p = _get_path_str(f).lower()
        basename = p.split("/")[-1]
        
        # 1. External phishing / unlock link traps (.url, .lnk, .website)
        if basename.endswith((".url", ".lnk", ".website")) and any(k in basename for k in ("password", "unlock", "key", "survey")):
            external_traps.append(basename)
        # 2. Local archive password note (.txt, .nfo)
        elif basename in PASSWORD_TRAP_FILENAMES or ("password" in basename and basename.endswith(".txt")):
            local_notes.append(basename)

    return len(external_traps) > 0, len(local_notes) > 0, (external_traps + local_notes)[:3]


def check_homoglyph_spoofing(text: Optional[str] = None) -> bool:
    """
    Detect suspicious Cyrillic-Latin homoglyph mixing within single words.
    Correctly exempts bilingual release conventions (2хRus, томI, TVOЁ).
    """
    if not text:
        return False

    words = re.findall(r"[A-Za-z\u0400-\u04FF\d]+", text)
    for word in words:
        # Exemptions for standard release tags and volume Roman numerals
        if re.match(r"^\d+[хx][A-Za-z]+$", word, re.IGNORECASE):
            continue
        if re.match(r"^(том|вып|ч|часть)[IVXLCDM]+$", word, re.IGNORECASE):
            continue
        if word in ("TVOЁ", "DVOЁ", "MVOЁ", "БеZ"):
            continue

        latin_chars = [c for c in word if ("A" <= c <= "Z") or ("a" <= c <= "z")]
        cyrillic_chars = [c for c in word if "\u0400" <= c <= "\u04FF"]

        if not latin_chars or not cyrillic_chars:
            continue

        # Must contain at least one visual lookalike character
        lookalikes = [c for c in cyrillic_chars if c in CYRILLIC_LOOKALIKES]
        if not lookalikes:
            continue

        # Must be primarily Latin with minority lookalike substitution (e.g. Вdrip, Cambridgе, .prо)
        ratio = len(latin_chars) / len(word)
        if ratio >= 0.60:
            return True

    return False


def evaluate_silver_invariants(
    name: Optional[str] = None,
    total_size: int = 0,
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

    has_ext_trap, has_loc_note, pw_files = check_password_trap(files)
    if has_ext_trap:
        reasons.append(ReasonCode.PASSWORD_TRAP_SUSPECTED)
        details["password_traps"] = pw_files
    elif has_loc_note:
        reasons.append(ReasonCode.LOCAL_PASSWORD_NOTE)
        details["local_password_notes"] = pw_files

    if check_homoglyph_spoofing(name):
        reasons.append(ReasonCode.HOMOGLYPH_PATH_SPOOFING)
        details["homoglyph"] = True

    return reasons, details
