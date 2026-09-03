from __future__ import annotations

import math


def extract_numeric_features(torrent: dict) -> dict:
    """Extract numeric features from raw torrent metadata.

    NO REGEX — the MLP learns patterns from raw data via TF-IDF.
    These numeric features complement the TF-IDF text features.
    """
    files = torrent.get("largest_files", []) or []
    file_count = torrent.get("file_count", 0) or 0
    total_size = torrent.get("total_size_bytes", 0) or 0

    # Basic stats
    avg_file_size = total_size / max(file_count, 1)
    file_sizes = [f.get("size", 0) for f in files if f.get("size", 0) > 0]
    max_file_size = max(file_sizes) if file_sizes else 0
    min_file_size = min(file_sizes) if file_sizes else 0

    # Extension/folder counts
    extensions = torrent.get("extensions", []) or []
    top_folders = torrent.get("top_folders", []) or []
    num_extensions = len(extensions)
    num_folders = len(top_folders)

    # Largest file ratio
    largest_file_ratio = max_file_size / max(total_size, 1)

    return {
        "file_count": file_count,
        "total_size_bytes": total_size,
        "avg_file_size": avg_file_size,
        "max_file_size": max_file_size,
        "min_file_size": min_file_size,
        "num_extensions": num_extensions,
        "num_folders": num_folders,
        "largest_file_ratio": largest_file_ratio,
    }


NUMERIC_FEATURE_NAMES = [
    "file_count",
    "total_size_bytes",
    "avg_file_size",
    "max_file_size",
    "min_file_size",
    "num_extensions",
    "num_folders",
    "largest_file_ratio",
]
