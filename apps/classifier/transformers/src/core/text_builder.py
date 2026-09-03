from __future__ import annotations

from .types import TorrentInput


def _parse_files(torrent: dict | TorrentInput) -> tuple[str, int, int, list]:
    """Parse common fields from torrent dict or TorrentInput."""
    if isinstance(torrent, TorrentInput):
        return torrent.name, torrent.file_count, torrent.total_size_bytes, torrent.files
    return (
        str(torrent.get("name", "")),
        torrent.get("file_count", 0),
        torrent.get("total_size", torrent.get("total_size_bytes", 0)),
        torrent.get("top_dirs", torrent.get("files", [])) or [],
    )


def build_input_text(torrent: dict | TorrentInput, config: dict | None = None) -> str:
    """Build plain-text representation from raw torrent metadata.

    Uses ONLY: name, file_count, total_size, extensions, top_folders, largest_files.
    No regex, no feature engineering — the model learns patterns from data.

    This function MUST be used by both training and inference
    to ensure identical text format.
    """
    cfg = (config or {}).get("text_builder", {})
    max_name_chars = cfg.get("max_name_chars", 300)

    name, file_count, total_size, raw_files = _parse_files(torrent)

    # Use pre-extracted metadata if available (from MCP server),
    # otherwise extract from raw file paths/dicts
    if isinstance(torrent, dict):
        extensions = torrent.get("extensions", [])
        top_folders = torrent.get("top_folders", [])
        largest_files = torrent.get("largest_files", [])
        # If no pre-extracted metadata, extract from files
        if not extensions and raw_files:
            extensions, top_folders, largest_files = _extract_metadata(raw_files)

    name = name[:max_name_chars]
    lines = [f"Name: {name}", f"Files: {file_count}  Size: {total_size}"]
    if extensions:
        lines.append(f"Extensions: {', '.join(str(e) for e in extensions[:10])}")
    if top_folders:
        lines.append(f"Top folders: {', '.join(str(f) for f in top_folders[:10])}")
    if largest_files:
        if isinstance(largest_files[0], dict):
            file_strs = [f"{f.get('name', '?')} ({f.get('size', 0)})" for f in largest_files[:3]]
        else:
            file_strs = [str(f) for f in largest_files[:3]]
        lines.append(f"Largest files: {', '.join(file_strs)}")
    return "\n".join(lines)


def _extract_metadata(raw_files) -> tuple[list[str], list[str], list[dict]]:
    """Extract extensions, top folders, and largest files from raw file paths."""
    if not raw_files or not isinstance(raw_files, list):
        return [], [], []

    extensions = []
    top_folders = set()
    file_entries = []

    for f in raw_files:
        if isinstance(f, dict):
            path = f.get("path", [])
            name = "/".join(str(p) for p in path) if isinstance(path, list) else str(path)
            size = f.get("length", f.get("size", 0))
        elif isinstance(f, list):
            name = "/".join(str(p) for p in f)
            size = 0
        elif isinstance(f, str):
            name = f
            size = 0
        else:
            continue

        # Extension
        dot = name.rfind(".")
        if dot >= 0:
            ext = name[dot:].lower()
            if ext not in extensions:
                extensions.append(ext)

        # Top folder
        slash = name.find("/")
        if slash > 0:
            top_folders.add(name[:slash])

        file_entries.append({"name": name.rsplit("/", 1)[-1] if "/" in name else name, "size": size})

    # Sort by size descending for largest files
    file_entries.sort(key=lambda x: x.get("size", 0), reverse=True)

    return extensions[:10], sorted(top_folders)[:10], file_entries[:3]
