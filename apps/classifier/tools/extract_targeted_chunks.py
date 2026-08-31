#!/usr/bin/env python3
"""
Extract targeted annotation chunks from the unlabeled pool for specific weak classes.
Produces 5 chunks of ~200 items each for parallel subagent labeling.

Priority:
  - Chunks 0-2: Applications-biased (setup, portable, installer keywords)
  - Chunk 3:    Anime-biased (fansub groups, JP broadcast, episode patterns)
  - Chunk 4:    Other-biased (ambiguous archives, single-file RAR/ZIP)
"""
from __future__ import annotations

import json
import logging
import random
import re
from pathlib import Path

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("extract_targeted_chunks")

# ── Freeze sets ──────────────────────────────────────────────────────────────
EVAL_PATHS = [
    "data/gold_pilot_v1/reference_eval_v1.jsonl",
    "data/manual_eval_set_balanced_2000.jsonl",
    "data/manual_eval_set_1000.jsonl",
    "data/baseline_v2/validation.jsonl",
    "data/baseline_v1/validation.jsonl",
]

EXISTING_LABEL_PATHS = [
    "data/production_pool/pooled_labeled.jsonl",
    "data/production_pool/subagent_annotated.jsonl",
    "data/baseline_v2/train.jsonl",
]

# ── Source pools (largest first) ─────────────────────────────────────────────
SOURCE_POOLS = [
    "data/unlabeled_pool.jsonl",          # 20k items
    "data/unlabeled_pool_60k.jsonl",      # 60k items
    "data/torrent_sample.jsonl",          # 400k-ish
    "data/natural_test_set_2000.jsonl",   # extra variety
    "data/production_pool/pooled_unlabeled.jsonl",
]

# ── Keyword filters ───────────────────────────────────────────────────────────
APP_KEYWORDS = re.compile(
    r"\b(setup|install|installer|portable|keygen|crack|patch|activat|license|serial|"
    r"software|adobe|autodesk|microsoft|office|windows|macos|linux|android|ios|"
    r"jetbrains|vmware|virtualbox|antivirus|driver|toolkit|utility|utilities|"
    r"plugin|extension|mod|hack|trainer|cheat\s?engine|repack(?!ed\s*game)|"
    r"v\d+\.\d+|x86|x64|64.?bit|32.?bit)\b",
    re.IGNORECASE,
)

ANIME_KEYWORDS = re.compile(
    r"\[(?:Erai-raws|SubsPlease|HorribleSubs|Judas|DKB|ASW|Commie|FFF|Coalgirls|"
    r"Anime\s*Time|NeoAE|Baha|ANi|VCB-Studio|Kawaiika-Raws|Golumpa|EMBER|SweetSub|"
    r"Lilith-Raws|NC-Raws|LoliHouse|Moozzi2|ReinForce|Kametsu|Yameii|ToonsHub|"
    r"Nekomoe|Tenshi|Ohys-Raws|Leopard-Raws)\]|"
    r"\b(AT-X|Tokyo\s*MX|BS11|Crunchyroll|Funimation|HIDIVE|"
    r"Episode\s+\d+|OVA|ONA|BD\s*(?:Remux|1080p|720p))\b|"
    r"(?:[\u3000-\u9fff\uF900-\uFAFF]){3,}",  # CJK characters
    re.IGNORECASE,
)

OTHER_KEYWORDS = re.compile(
    r"^\S+\.(rar|zip|7z|tar|gz|001)$|"
    r"\b(part\d+|vol\d+|\d+of\d+|sample|preview|ebook|book|course|tutorial|"
    r"lesson|lecture|training|xxx|adult|porn|hentai|onlyfans|jav|fc2)\b",
    re.IGNORECASE,
)

GAME_EXCLUDE = re.compile(
    r"\b(fitgirl|codex|skidrow|reloaded|plaza|dodi|rune|gog|"
    r"\.nsp|\.xci|\.cia|\.3ds|\.vpk|\.wbfs|\.cso|\.nds|\.gba|\.iso\b)\b",
    re.IGNORECASE,
)


def load_frozen_hashes() -> set[str]:
    frozen: set[str] = set()
    for p in EVAL_PATHS + EXISTING_LABEL_PATHS:
        path = Path(p)
        if path.exists():
            with open(path, encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        obj = json.loads(line)
                        ih = obj.get("infohash")
                        if ih:
                            frozen.add(str(ih).lower())
                    except Exception:
                        pass
    logger.info("Loaded %d frozen hashes.", len(frozen))
    return frozen


def load_candidates(frozen: set[str]) -> list[dict]:
    seen: set[str] = set(frozen)
    candidates: list[dict] = []

    for pool_path in SOURCE_POOLS:
        path = Path(pool_path)
        if not path.exists():
            logger.warning("Pool not found: %s", pool_path)
            continue
        added = 0
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except Exception:
                    continue
                ih = str(obj.get("infohash", "")).lower()
                if not ih or ih in seen:
                    continue
                name = str(obj.get("name", "")).strip()
                if not name:
                    continue
                seen.add(ih)
                # Normalize files
                files = obj.get("files", obj.get("top_dirs", [])) or []
                if isinstance(files, list) and files and isinstance(files[0], dict):
                    files = [
                        "/".join(str(x) for x in f.get("path", [])) if isinstance(f.get("path"), list) else str(f.get("path", ""))
                        for f in files
                    ]
                elif isinstance(files, list) and files and isinstance(files[0], list):
                    files = ["/".join(str(x) for x in f) for f in files]
                candidates.append({
                    "infohash": ih,
                    "name": name,
                    "file_count": int(obj.get("file_count", len(files) if files else 1)),
                    "total_size_bytes": int(obj.get("total_size_bytes", obj.get("total_size", 0))),
                    "files": [str(f) for f in files if isinstance(f, str)][:50],
                })
                added += 1
        logger.info("Loaded %d new candidates from %s", added, pool_path)

    logger.info("Total unique candidates: %d", len(candidates))
    return candidates


def score_app(item: dict) -> float:
    text = item["name"] + " " + " ".join(item["files"][:20])
    score = len(APP_KEYWORDS.findall(text)) * 2.0
    if GAME_EXCLUDE.search(item["name"]):
        score -= 5.0
    return score


def score_anime(item: dict) -> float:
    text = item["name"] + " " + " ".join(item["files"][:10])
    return float(len(ANIME_KEYWORDS.findall(text)) * 3.0)


def score_other(item: dict) -> float:
    name = item["name"]
    fc = item.get("file_count", 1)
    score = 0.0
    if OTHER_KEYWORDS.search(name):
        score += 3.0
    if fc == 1:
        score += 1.0
    if APP_KEYWORDS.search(name) or ANIME_KEYWORDS.search(name):
        score -= 2.0
    return score


def extract_chunks(candidates: list[dict], chunk_size: int = 200) -> dict[str, list[dict]]:
    random.seed(42)

    # Score all candidates
    for c in candidates:
        c["_app_score"] = score_app(c)
        c["_anime_score"] = score_anime(c)
        c["_other_score"] = score_other(c)

    # Sort pools
    app_sorted = sorted(candidates, key=lambda x: (-x["_app_score"], random.random()))
    anime_sorted = sorted(candidates, key=lambda x: (-x["_anime_score"], random.random()))
    other_sorted = sorted(candidates, key=lambda x: (-x["_other_score"], random.random()))

    used: set[str] = set()

    def pick(pool: list[dict], n: int) -> list[dict]:
        picked = []
        for c in pool:
            if c["infohash"] not in used:
                picked.append(c)
                used.add(c["infohash"])
                if len(picked) >= n:
                    break
        return picked

    chunks = {
        "al_targeted_chunk_0": pick(app_sorted, chunk_size),   # Apps heavy
        "al_targeted_chunk_1": pick(app_sorted, chunk_size),   # Apps medium
        "al_targeted_chunk_2": pick(app_sorted, chunk_size),   # Apps light + mixed
        "al_targeted_chunk_3": pick(anime_sorted, chunk_size), # Anime heavy
        "al_targeted_chunk_4": pick(other_sorted, chunk_size), # Other/ambiguous
    }

    for name, chunk in chunks.items():
        logger.info("Chunk %s: %d items (avg app=%.1f, anime=%.1f, other=%.1f)",
                    name,
                    len(chunk),
                    sum(c["_app_score"] for c in chunk) / max(len(chunk), 1),
                    sum(c["_anime_score"] for c in chunk) / max(len(chunk), 1),
                    sum(c["_other_score"] for c in chunk) / max(len(chunk), 1))
    return chunks


def main():
    out_dir = Path("data")

    frozen = load_frozen_hashes()
    candidates = load_candidates(frozen)

    if len(candidates) < 1000:
        logger.warning("Only %d candidates available — pool may be exhausted.", len(candidates))

    chunks = extract_chunks(candidates, chunk_size=200)

    for chunk_name, items in chunks.items():
        out_path = out_dir / f"{chunk_name}.jsonl"
        with open(out_path, "w", encoding="utf-8") as f:
            for item in items:
                # Remove scoring keys before writing
                clean = {k: v for k, v in item.items() if not k.startswith("_")}
                f.write(json.dumps(clean, ensure_ascii=False) + "\n")
        logger.info("Wrote %d items to %s", len(items), out_path)

    logger.info("Done. %d chunks extracted.", len(chunks))


if __name__ == "__main__":
    main()
