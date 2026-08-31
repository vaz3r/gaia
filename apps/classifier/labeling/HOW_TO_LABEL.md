# How to Label Torrents

## Setup
1. Open your AI chat (Google AI Studio / DeepSeek Chat)
2. Copy the entire content of `SYSTEM_PROMPT.md` and paste it as the system prompt

## Per-Batch Workflow

1. Pick a category from `labeling/batches/` (start with whichever you prefer)
2. Open a batch file (e.g. `labeling/batches/anime/batch_001.json`)
3. Copy the entire JSON array
4. Paste it into the AI chat as a user message
5. The AI will return a labeled JSON array
6. Copy the output
7. Save it to `labeling/labeled/{category}/batch_001_labeled.json`
8. Repeat for all batches

## Output Format

The labeled file should contain ONLY the JSON array returned by the AI. No extra text, no markdown fences. Example:

```json
[
  {"infohash":"abc123...","label_category":"Anime","confidence":"high","reason":"Fansub tag [Erai-raws] present"},
  {"infohash":"def456...","label_category":"Anime","confidence":"high","reason":"Naruto franchise, S01 format"}
]
```

## Tips
- **Confidence**: Mark as "low" if the AI is uncertain. We can filter these out later.
- **Batch size**: 100 items per batch. If the AI struggles, switch to 50-item batches.
- **Double-check**: Skim the output. If the AI mislabeled obvious items, correct them manually.
- **Progress**: Files are named `batch_001.json`, `batch_002.json`, etc. Just work through them sequentially.

## Categories to Label (target: 5000 each)

| Category | Sub-queries | Approx batches |
|---|---|---|
| Anime | fansub tags + franchise names | ~50 |
| Applications | Adobe + misc software | ~50 |
| Games | scene releases + console ROMs | ~50 |
| Documentaries | documentary markers | ~30 |
| Music | discography/album/FLAC | ~50 |
| Movies | quality + year | ~50 |
| Television | SxxExx / Season | ~50 |

## After Labeling

When you've labeled enough batches, run:

```bash
python3 labeling/merge_labeled.py
```

This will combine all labeled batches into `data/human_labeled_v2/merged.jsonl` ready for training.
