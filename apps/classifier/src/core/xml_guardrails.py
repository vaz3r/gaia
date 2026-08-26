from __future__ import annotations

import re
import xml.etree.ElementTree as ET
import xml.sax.saxutils as saxutils

from .types import ALLOWED_CATEGORIES, ClassificationResult, TorrentInput


def safe_xml_text(value: str, max_len: int) -> str:
    value = value[:max_len]
    return saxutils.escape(value, {'"': "&quot;", "'": "&apos;"})


def build_torrent_xml(torrent: TorrentInput, config: dict | None = None) -> str:
    cfg = (config or {}).get("prompt", {})
    max_name = cfg.get("max_torrent_name_chars", 500)
    max_files = cfg.get("max_input_files", 20)
    max_file_chars = cfg.get("max_file_name_chars", 200)

    name = safe_xml_text(torrent.name, max_name)
    infohash = safe_xml_text(torrent.infohash, 64)
    files = torrent.files[:max_files]

    file_elements = []
    for f in files:
        if isinstance(f, list):
            f = str(f[0])
        file_elements.append(f"  <file>{safe_xml_text(f, max_file_chars)}</file>")
    files_block = "\n".join(file_elements)

    return (
        "<torrent>\n"
        f"  <infohash>{infohash}</infohash>\n"
        f"  <name>{name}</name>\n"
        f"  <file_count>{torrent.file_count}</file_count>\n"
        f"  <total_size_bytes>{torrent.total_size_bytes}</total_size_bytes>\n"
        f"  <files>\n{files_block}\n  </files>\n"
        "</torrent>"
    )


def parse_classification_xml(text: str) -> ClassificationResult | None:
    text = text.strip()
    text = re.sub(r"```(?:xml)?\s*", "", text, flags=re.IGNORECASE)
    text = re.sub(r"```\s*$", "", text, flags=re.IGNORECASE)
    text = text.strip()

    # Try direct parse
    try:
        root = ET.fromstring(text)
        return _extract_from_root(root)
    except ET.ParseError:
        pass

    # Try with closing tag (model may omit it)
    match = re.search(r"<classification>.*?</classification>", text, re.DOTALL)
    if match:
        try:
            root = ET.fromstring(match.group(0))
            return _extract_from_root(root)
        except ET.ParseError:
            pass

    # Fallback: extract category and confidence via regex (handles truncated XML)
    cat_match = re.search(r"<category>\s*(\w+)\s*</category>", text)
    conf_match = re.search(r"<confidence>\s*([\d.]+)\s*</confidence>", text)
    if cat_match and conf_match:
        cat = cat_match.group(1)
        try:
            conf = float(conf_match.group(1))
        except ValueError:
            return None
        if cat in ALLOWED_CATEGORIES and 0.0 <= conf <= 1.0:
            return ClassificationResult(category=cat, confidence=round(conf, 4))

    return None


def _extract_from_root(root: ET.Element) -> ClassificationResult | None:
    if root.tag != "classification":
        return None

    cat = (root.findtext("category") or "").strip()
    conf_str = (root.findtext("confidence") or "").strip()

    if cat not in ALLOWED_CATEGORIES:
        return None

    try:
        conf = float(conf_str)
    except (ValueError, TypeError):
        return None

    if not (0.0 <= conf <= 1.0):
        return None

    return ClassificationResult(category=cat, confidence=round(conf, 4))


RETRY_SYSTEM_SUFFIX = (
    "\nYou must output exactly one XML block. No other text. "
    "Do not include markdown fences."
)


def build_retry_system_prompt(original_system: str) -> str:
    return original_system + RETRY_SYSTEM_SUFFIX
