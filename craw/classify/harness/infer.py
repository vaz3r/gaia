#!/usr/bin/env python3
"""Batch inference via ollama HTTP API."""

import argparse
import json
import sys
import time
import requests

CATEGORIES = {"Movies","Television","Games","Music","Applications","Anime","Documentaries","Other","Unwanted"}

SYSTEM_PROMPT = """You are a torrent classifier. Given a torrent's name, file count, size, and top directories, classify it.

Respond ONLY with valid JSON in this exact format:
{"category": "<one of: Movies, Television, Games, Music, Applications, Anime, Documentaries, Other, Unwanted>", "keep": <true or false>, "confidence": <0.0 to 1.0>, "explicit": <true or false>}

Rules:
- Unwanted: spam, malware, crypto miners, broken/fake files, password-protected archives, scene rips of unwanted content. keep=false.
- If regex_explicit is "yes", set explicit=true and category=Unwanted, keep=false.
- Movies: full films, cam/ts/hdrip/screeener rips. keep=true.
- Television: TV episodes, seasons, series packs. keep=true.
- Music: albums, singles, FLAC/MP3, music videos. keep=true.
- Games: PC/console game ISOs, ROMs, cracks. keep=true.
- Applications: software, tools, installers. keep=true.
- Anime: Japanese animation, subs/dubs. keep=true.
- Documentaries: documentary films. keep=true.
- Other: anything that doesn't fit above categories but is legitimate content. keep=true.
- confidence: how confident you are (0.0-1.0)."""

def format_prompt(row):
    top_dirs = ", ".join(row.get("top_dirs", [])[:5])
    return (
        f"Name: {row['name']}\n"
        f"Files: {row['file_count']}  Size: {row['total_size']}\n"
        f"Top dirs: {top_dirs}\n"
        f"Regex explicit: {'yes' if row.get('regex_explicit') else 'no'}"
    )

def parse_response(raw):
    """Parse JSON from model output. Returns None on malformed."""
    raw = raw.strip()
    # Strip thinking tags if present
    if "<|channel>thought" in raw:
        parts = raw.split("<channel|>")
        if len(parts) > 1:
            raw = parts[-1].strip()
    try:
        obj = json.loads(raw)
    except json.JSONDecodeError:
        # Try to extract JSON from surrounding text
        import re
        m = re.search(r'\{[^{}]*\}', raw)
        if m:
            try:
                obj = json.loads(m.group())
            except json.JSONDecodeError:
                return None
        else:
            return None
    if not isinstance(obj, dict):
        return None
    cat = obj.get("category")
    if cat not in CATEGORIES:
        return None
    keep = obj.get("keep")
    if not isinstance(keep, bool):
        return None
    conf = obj.get("confidence")
    if not isinstance(conf, (int, float)) or not (0.0 <= conf <= 1.0):
        return None
    expl = obj.get("explicit")
    if not isinstance(expl, bool):
        return None
    return {"category": cat, "keep": keep, "confidence": round(conf, 3), "explicit": expl}

def query_ollama(host, model, prompt, system, temperature=0, num_predict=128):
    """Call ollama generate API."""
    resp = requests.post(
        f"{host}/api/generate",
        json={
            "model": model,
            "prompt": prompt,
            "system": system,
            "options": {
                "temperature": temperature,
                "num_predict": num_predict,
            },
            "stream": False,
        },
        timeout=600,
    )
    resp.raise_for_status()
    return resp.json()["response"]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, help="Ollama model name")
    parser.add_argument("--sample", required=True, help="Path to sample JSON")
    parser.add_argument("--output", required=True, help="Output path for predictions JSON")
    parser.add_argument("--ollama-host", default="http://localhost:11434")
    parser.add_argument("--n-predict", type=int, default=128)
    parser.add_argument("--temp", type=float, default=0.0)
    args = parser.parse_args()

    with open(args.sample) as f:
        sample = json.load(f)

    results = []
    n_total = len(sample)
    t0 = time.time()

    for i, row in enumerate(sample):
        prompt = format_prompt(row)
        try:
            text = query_ollama(
                args.ollama_host, args.model, prompt, SYSTEM_PROMPT,
                temperature=args.temp, num_predict=args.n_predict,
            )
        except Exception as e:
            text = f"ERROR: {e}"

        parsed = parse_response(text)
        results.append({
            "idx": row["id"],
            "infohash": row.get("infohash", ""),
            "name": row["name"],
            "raw_output": text,
            "parsed": parsed,
        })

        if (i + 1) % 10 == 0 or i == n_total - 1:
            elapsed = time.time() - t0
            rate = (i + 1) / elapsed if elapsed > 0 else 0
            malformed = sum(1 for r in results if r["parsed"] is None)
            print(
                f"[{i+1}/{n_total}] {rate:.1f} it/s | malformed: {malformed} | elapsed: {elapsed:.0f}s",
                flush=True,
            )

    with open(args.output, "w") as f:
        json.dump(results, f, indent=2)

    malformed = sum(1 for r in results if r["parsed"] is None)
    elapsed = time.time() - t0
    print(f"\nDone. {n_total} predictions in {elapsed:.0f}s ({n_total/elapsed:.1f} it/s)")
    print(f"Malformed: {malformed}/{n_total} ({malformed/n_total*100:.1f}%)")

if __name__ == "__main__":
    main()
