from __future__ import annotations

from .types import TorrentInput


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

    name = name[:max_name_chars]
    top_dirs = files[:max_files]
    dirs_str = ", ".join(d[:max_file_chars] for d in top_dirs)

    return (
        f"Name: {name}\n"
        f"Files: {file_count}  Size: {total_size}\n"
        f"Top dirs: {dirs_str}"
    )
