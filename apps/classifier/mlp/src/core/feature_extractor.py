from __future__ import annotations

import math


def extract_numeric_features(torrent: dict) -> dict:
    """Extract numeric features from raw torrent metadata.

    Uses files_raw when available for richer statistics.
    NO regex — the MLP learns patterns from raw data via TF-IDF.
    These numeric features complement the TF-IDF text features.
    """
    files_raw = torrent.get("files_raw", []) or []
    largest_files = torrent.get("largest_files", []) or []
    file_count = torrent.get("file_count", 0) or 0
    total_size = torrent.get("total_size_bytes", 0) or 0

    # Parse all file sizes from files_raw if available
    all_sizes = []
    all_exts = set()
    all_folders = set()

    if files_raw and isinstance(files_raw, list) and len(files_raw) > 0 and isinstance(files_raw[0], dict):
        for f in files_raw:
            length = f.get("length", 0)
            if length > 0:
                all_sizes.append(length)
            # Extract extension from path
            path = f.get("path", [])
            if isinstance(path, list) and path:
                name = path[-1]
                dot = name.rfind(".")
                if dot >= 0:
                    ext = name[dot + 1:].lower()
                    if ext:
                        all_exts.add(ext)
                # Extract top-level folder
                if len(path) > 1:
                    all_folders.add(path[0])
    else:
        # Fallback to largest_files
        for f in largest_files:
            if isinstance(f, dict):
                size = f.get("size", 0)
                if size > 0:
                    all_sizes.append(size)

    # Compute statistics from all sizes
    avg_file_size = total_size / max(file_count, 1)
    max_file_size = max(all_sizes) if all_sizes else 0
    min_file_size = min(s for s in all_sizes if s > 0) if all_sizes else 0
    largest_file_ratio = max_file_size / max(total_size, 1)

    # Extension and folder counts
    num_extensions = len(all_exts) if all_exts else len(torrent.get("extensions", []) or [])
    num_folders = len(all_folders) if all_folders else len(torrent.get("top_folders", []) or [])

    return {
        # Original features
        "file_count": file_count,
        "total_size_bytes": total_size,
        "avg_file_size": avg_file_size,
        "max_file_size": max_file_size,
        "min_file_size": min_file_size,
        "num_extensions": num_extensions,
        "num_folders": num_folders,
        "largest_file_ratio": largest_file_ratio,
        # Log-transformed features (tames extreme skew)
        "log_file_count": math.log1p(file_count),
        "log_total_size": math.log1p(total_size),
        "log_avg_file_size": math.log1p(avg_file_size),
        "log_max_file_size": math.log1p(max_file_size),
        # Ratio features
        "max_to_total_ratio": max_file_size / max(total_size, 1),
        "min_to_max_ratio": min_file_size / max(max_file_size, 1),
        "ext_per_file": num_extensions / max(file_count, 1),
    }


NUMERIC_FEATURE_NAMES = [
    # Original
    "file_count",
    "total_size_bytes",
    "avg_file_size",
    "max_file_size",
    "min_file_size",
    "num_extensions",
    "num_folders",
    "largest_file_ratio",
    # Log-transformed
    "log_file_count",
    "log_total_size",
    "log_avg_file_size",
    "log_max_file_size",
    # Ratios
    "max_to_total_ratio",
    "min_to_max_ratio",
    "ext_per_file",
]
