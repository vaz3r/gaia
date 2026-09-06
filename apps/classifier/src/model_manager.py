import os
import sys
import json
import time
from pathlib import Path
from typing import Dict, Any, List, Optional

MODELS_DIR = Path(__file__).parent.parent / "models"
ACTIVE_MODEL_JSON = MODELS_DIR / "active_model.json"
DEFAULT_MODEL_PATH = MODELS_DIR / "torrent_classifier_v2.joblib"


def init_active_model_if_missing():
    """Ensure active_model.json exists with baseline v2."""
    MODELS_DIR.mkdir(parents=True, exist_ok=True)
    if not ACTIVE_MODEL_JSON.exists():
        initial_data = {
            "version": "v2",
            "filename": "torrent_classifier_v2.joblib",
            "model_path": str(DEFAULT_MODEL_PATH),
            "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "metrics": {
                "macro_f1": 0.9040,
                "accuracy": 0.9073,
                "per_class_f1": {
                    "Adult": 0.932,
                    "Anime": 0.966,
                    "Applications": 0.887,
                    "Audiobooks": 0.865,
                    "Books & Learning": 0.957,
                    "Documentaries": 0.835,
                    "Games": 0.923,
                    "Movies": 0.871,
                    "Music": 0.932,
                    "Television": 0.872
                }
            },
            "history": [
                {
                    "version": "v2",
                    "filename": "torrent_classifier_v2.joblib",
                    "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                    "macro_f1": 0.9040
                }
            ]
        }
        with open(ACTIVE_MODEL_JSON, "w", encoding="utf-8") as f:
            json.dump(initial_data, f, indent=2)


def get_active_model_info() -> Dict[str, Any]:
    init_active_model_if_missing()
    try:
        with open(ACTIVE_MODEL_JSON, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {"version": "v2", "filename": "torrent_classifier_v2.joblib", "model_path": str(DEFAULT_MODEL_PATH)}


def get_active_model_path() -> Path:
    info = get_active_model_info()
    path = MODELS_DIR / info.get("filename", "torrent_classifier_v2.joblib")
    if path.exists():
        return path
    if DEFAULT_MODEL_PATH.exists():
        return DEFAULT_MODEL_PATH
    raise FileNotFoundError(f"Active model not found at {path} or {DEFAULT_MODEL_PATH}")


def list_available_models() -> List[Dict[str, Any]]:
    init_active_model_if_missing()
    info = get_active_model_info()
    active_filename = info.get("filename", "torrent_classifier_v2.joblib")
    history_entries = info.get("history", [])
    history_by_file = {h.get("filename"): h for h in history_entries if isinstance(h, dict) and h.get("filename")}

    models = []
    for p in sorted(MODELS_DIR.glob("torrent_classifier_*.joblib")):
        size_mb = p.stat().st_size / (1024 * 1024)
        fname = p.name
        # extract version string
        v_name = fname.replace("torrent_classifier_", "").replace(".joblib", "")
        is_active = (fname == active_filename)

        # Baseline or cached metrics
        macro_f1 = None
        accuracy = None
        num_samples = None
        trained_at = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(p.stat().st_mtime))

        if is_active and info.get("metrics"):
            m = info["metrics"]
            macro_f1 = m.get("macro_f1")
            accuracy = m.get("accuracy")

        hist = history_by_file.get(fname)
        if hist:
            if macro_f1 is None and "macro_f1" in hist:
                macro_f1 = hist["macro_f1"]
            if "metrics" in hist and isinstance(hist["metrics"], dict):
                hm = hist["metrics"]
                macro_f1 = hm.get("macro_f1", macro_f1)
                accuracy = hm.get("accuracy", accuracy)

        # Use cached metadata from active_model.json or baseline defaults (avoiding 17MB joblib.load on each HTTP call)
        if v_name == "v2":
            if macro_f1 is None:
                macro_f1 = 0.9040
            if accuracy is None:
                accuracy = 0.9073
            if num_samples is None:
                num_samples = 30869

        models.append({
            "version": v_name,
            "filename": fname,
            "size_mb": round(size_mb, 2),
            "is_active": is_active,
            "macro_f1": macro_f1,
            "accuracy": accuracy,
            "num_samples": num_samples,
            "trained_at": trained_at,
            "modified_at": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(p.stat().st_mtime))
        })
    return models



def activate_model(
    version: str,
    filename: str,
    metrics: Dict[str, Any],
    canary_stats: Optional[Dict[str, Any]] = None
) -> Dict[str, Any]:
    """Atomically activate a new model version and record in history."""
    init_active_model_if_missing()
    with open(ACTIVE_MODEL_JSON, "r", encoding="utf-8") as f:
        data = json.load(f)

    history = data.get("history", [])
    history.append({
        "version": version,
        "filename": filename,
        "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "macro_f1": metrics.get("macro_f1", 0.0),
        "canary_stats": canary_stats
    })

    new_info = {
        "version": version,
        "filename": filename,
        "model_path": str(MODELS_DIR / filename),
        "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "metrics": metrics,
        "canary_stats": canary_stats,
        "history": history[-15:]  # Keep last 15 in history
    }

    temp_json = ACTIVE_MODEL_JSON.with_suffix(".tmp")
    with open(temp_json, "w", encoding="utf-8") as f:
        json.dump(new_info, f, indent=2)
    temp_json.replace(ACTIVE_MODEL_JSON)
    return new_info


def rollback_to_version(target_version: str) -> Dict[str, Any]:
    """Manually rollback active_model.json to a specific existing version artifact."""
    init_active_model_if_missing()
    models = list_available_models()
    matching = [m for m in models if m["version"] == target_version or m["filename"] == target_version]
    if not matching:
        available = [m["version"] for m in models]
        raise ValueError(f"Version '{target_version}' not found. Available: {available}")

    target = matching[0]
    target_path = MODELS_DIR / target["filename"]

    with open(ACTIVE_MODEL_JSON, "r", encoding="utf-8") as f:
        data = json.load(f)

    history = data.get("history", [])

    # Find the target model metrics from history or list_available_models
    target_macro_f1 = target.get("macro_f1") or 0.9040
    target_accuracy = target.get("accuracy") or 0.9073
    target_metrics = {
        "macro_f1": target_macro_f1,
        "accuracy": target_accuracy,
        "per_class_f1": data.get("metrics", {}).get("per_class_f1", {})
    }

    # If target has full metrics recorded in an earlier history record, restore it
    for h in reversed(history):
        if h.get("filename") == target["filename"] and "metrics" in h and isinstance(h["metrics"], dict):
            target_metrics = h["metrics"]
            break

    history.append({
        "version": target["version"],
        "filename": target["filename"],
        "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "macro_f1": target_macro_f1,
        "rollback": True
    })

    new_info = {
        "version": target["version"],
        "filename": target["filename"],
        "model_path": str(target_path),
        "activated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "metrics": target_metrics,
        "history": history[-15:]
    }

    temp_json = ACTIVE_MODEL_JSON.with_suffix(".tmp")
    with open(temp_json, "w", encoding="utf-8") as f:
        json.dump(new_info, f, indent=2)
    temp_json.replace(ACTIVE_MODEL_JSON)
    return new_info


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Model Manager & Rollback Tool")
    subparsers = parser.add_subparsers(dest="action")

    subparsers.add_parser("list", help="List available model versions")
    subparsers.add_parser("current", help="Show current active model")

    rb = subparsers.add_parser("rollback", help="Rollback to a previous version")
    rb.add_argument("--version", required=True, help="Version name (e.g. v2, v3)")

    args = parser.parse_args()
    if args.action == "list":
        print(json.dumps(list_available_models(), indent=2))
    elif args.action == "rollback":
        res = rollback_to_version(args.version)
        print(f"Successfully rolled back to: {res['version']} ({res['filename']})")
    else:
        print(json.dumps(get_active_model_info(), indent=2))
