"""
Semantic Threat Feature Extraction for Torrent Files and Metadata.
Extracts token-level, n-gram, and subword threat patterns for detecting
deceptive media trojans, malicious address publishers, fake codecs,
and phishing credential traps.
"""
import re
import math
from typing import Dict, List, Any, Optional
from collections import Counter

# Deceptive player / codec tokens commonly used in video/audio spoofing
DECEPTIVE_PLAYER_TOKENS = re.compile(
    r"(?i)\b(xvid|divx|codec|codec[-_]?pack|k[-_]?lite|vlc[-_]?setup|player[-_]?setup|"
    r"movie[-_]?player|video[-_]?player|flash[-_]?player|hd[-_]?player|media[-_]?player|"
    r"torrent[-_]?player|play[-_]?movie|watch[-_]?hd)\b"
)

# Gateway / malicious address publisher tokens (frequent in East Asian & Russian releases)
ADDRESS_PUBLISHER_TOKENS = re.compile(
    r"(?i)(地址发布器|最新网址|备用网址|全新改版|发布器|防迷路|社区新网址|网址导航|"
    r"回家地址|官网地址|永久地址|发布页|address[-_]?publisher|new[-_]?domain|"
    r"mirror[-_]?finder|url[-_]?updater)"
)

# Phishing and survey trap tokens
PHISHING_TRAP_TOKENS = re.compile(
    r"(?i)\b(password[-_]?unlock|unlock[-_]?password|key[-_]?generator|serial[-_]?generator|"
    r"activation[-_]?key|license[-_]?generator|crack[-_]?download|verify[-_]?to[-_]?unlock|"
    r"survey[-_]?unlock|survey[-_]?bypass|password[-_]?txt)\b"
)

# Bogus viewer/reader executables in document or audiobook releases
TROJAN_VIEWER_TOKENS = re.compile(
    r"(?i)\b(pdf|epub|mobi|book|comic|cbr|cbz|audio|voice)[-_]?(reader|viewer|player|opener)\.exe$"
)

# Suspicious mobile trojan tokens in APKs
SUSPICIOUS_APK_TOKENS = re.compile(
    r"(?i)(色游|直播|破解版|破解器|外挂|看片|福利|社区|发布器)"
)

# Dangerous dropper script extensions
SCRIPT_DROPPER_EXTS = {".bat", ".cmd", ".vbs", ".ps1", ".scr", ".pif", ".hta", ".cpl", ".reg"}


def extract_semantic_features(
    name: Optional[str] = None,
    files: Optional[List[Dict[str, Any]]] = None,
    category: Optional[str] = None,
) -> Dict[str, float]:
    """
    Extracts high-dimensional semantic and lexical threat signals.
    """
    cat_lower = (category or "").strip().lower()
    name_clean = (name or "").strip()
    
    # Collect file paths
    file_paths: List[str] = []
    if files:
        for f in files:
            if isinstance(f, dict):
                p_val = f.get("path") or f.get("name") or ""
                if isinstance(p_val, list):
                    file_paths.append("/".join(str(seg) for seg in p_val))
                else:
                    file_paths.append(str(p_val))
    
    all_text = name_clean + " " + " ".join(file_paths)
    
    # 1. Deceptive Player / Codec Token Signal
    has_deceptive_player = 1.0 if DECEPTIVE_PLAYER_TOKENS.search(all_text) else 0.0

    # 2. Address Publisher Signal
    has_address_publisher = 1.0 if ADDRESS_PUBLISHER_TOKENS.search(all_text) else 0.0

    # 3. Phishing / Trap Signal
    has_phishing_trap = 1.0 if PHISHING_TRAP_TOKENS.search(all_text) else 0.0

    # 4. Trojan Viewer Signal
    has_trojan_viewer = 0.0
    for p in file_paths:
        if TROJAN_VIEWER_TOKENS.search(p):
            has_trojan_viewer = 1.0
            break

    # 5. Hidden Script Dropper in Non-Software
    has_script_dropper = 0.0
    cat_is_software = any(c in cat_lower for c in ("application", "game"))
    if not cat_is_software:
        for p in file_paths:
            ext = "." + p.rsplit(".", 1)[-1].lower() if "." in p else ""
            if ext in SCRIPT_DROPPER_EXTS:
                has_script_dropper = 1.0
                break

    # 6. Trojan APK Signal
    has_trojan_apk = 0.0
    for p in file_paths:
        if p.lower().endswith(".apk"):
            if SUSPICIOUS_APK_TOKENS.search(p) or not cat_is_software:
                has_trojan_apk = 1.0
                break

    # 7. Obfuscated / Random Payload Entropy
    max_payload_entropy = 0.0
    for p in file_paths:
        ext = "." + p.rsplit(".", 1)[-1].lower() if "." in p else ""
        if ext in {".exe", ".scr", ".pif", ".apk"}:
            basename = p.rsplit("/", 1)[-1].rsplit("\\", 1)[-1]
            if len(basename) > 4:
                length = len(basename)
                counts = Counter(basename)
                ent = -sum((c / length) * math.log2(c / length) for c in counts.values())
                if ent > max_payload_entropy:
                    max_payload_entropy = ent

    return {
        "has_deceptive_player_token": has_deceptive_player,
        "has_address_publisher_token": has_address_publisher,
        "has_phishing_trap_token": has_phishing_trap,
        "has_trojan_viewer_token": has_trojan_viewer,
        "has_script_dropper_token": has_script_dropper,
        "has_trojan_apk_token": has_trojan_apk,
        "payload_basename_entropy": float(max_payload_entropy),
    }
