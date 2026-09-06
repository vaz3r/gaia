# DeepSeek Torrent Classifier

Standalone torrent classifier using DeepSeek's free API (no API key, no credits).

## Requirements

- Python 3.9+
- DeepSeek account (free chat.deepseek.com account works)
- PostgreSQL access to `labeled_results` table

## Setup

```bash
cd apps/classifier/deepseek
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
playwright install chromium   # one-time
python -m deepseek.auth       # sign in once in browser
```

## Usage

```bash
python classify.py              # one batch of 50
python classify.py --loops 10   # 10 batches
python classify.py --batch 100  # batches of 100
```

## How It Works

1. Fetches unclassified torrents from PostgreSQL
2. Sends metadata (name, files, extensions, size) to DeepSeek with classification prompt
3. Parses JSON response, validates each result
4. Records valid classifications in `labeled_results` table (`source='deepseek'`)
5. Skips malformed items instead of rejecting the batch

## Session Management

- First run opens a browser for sign-in
- Session is cached in `session/session.json`
- Automatic headless refresh for ~6 hours
- If session expires, re-run `python -m deepseek.auth`

## Files

| File | Purpose |
|------|---------|
| `classify.py` | Classification script |
| `deepseek/` | Core library (auth, client, PoW solver) |
| `session/` | Auth token + Chrome profile (gitignored) |
| `requirements.txt` | Dependencies |
