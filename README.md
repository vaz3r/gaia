# gaia

BitTorrent DHT crawler platform: a DHT crawler indexing movie/TV torrents into
PostgreSQL, with a monitoring pipeline, a search + admin API, and a React
dashboard — deployed behind a WireGuard tunnel.

## Services

| Service | Container | Purpose |
|---|---|---|
| `gluetun` | `gluetun` | WireGuard tunnel; all crawler egress via public IP |
| `crawler` | `crawler` | DHT sampler + metadata fetcher; persists to Postgres, writes a full 30s monitoring snapshot |
| `postgres` | `dht-postgres` | Single store (`pgvector/pgvector:pg16`): torrents, scanned, crawl_stats_history, app_config |
| `redis` | `dht-redis` | Crawler cross-instance dedup + dead-peer cache |
| `api` | `gaia-api` | Express (TS) — `/api/search` (pg_trgm) + `/api/admin` (monitoring, config) + `/health` |
| `dashboard` | `gaia-dashboard` | React/Vite UI, served by nginx, proxies `/api` to the API |

## Quick start

```sh
cp .env.example .env   # fill in WireGuard keys + POSTGRES_PASSWORD
docker compose up -d --build
```

- **Dashboard**: `http://<host>:8080`
- **API**: `http://<host>:3000`
- **Crawler logs**: `docker compose logs -f crawler`

## API endpoints

- `GET /health` — postgres / redis / crawler / api status
- `GET /api/admin/monitor/latest` — most recent 30s snapshot (all crawl + system metrics)
- `GET /api/admin/monitor/history?metric=<col>&range=<5m|30m|1h|6h|24h|7d>` — raw series
- `GET /api/admin/monitor/rates?metric=<col>&range=` — per-hour rate (LAG window function)
- `GET /api/admin/monitor/failures?range=` — fetch failures by reason
- `GET /api/admin/monitor/system?kind=<network|memory|cpu|disk|loadavg>&range=` — system series
- `GET/PUT/DELETE /api/admin/config/:key` — key/value config (JSONB)
- `GET /api/search?q=<query>&sort=<relevance|newest|largest|name>&order=<asc|desc>&size_min=&size_max=&limit=&from=` — fuzzy search

## Crawler CLI

`crawler run --pg <url> --instances 8 --scale 1 ...` (see `crawler/README.md`).
Other commands now target Postgres: `query`, `purge`, `snapshot` (pg_dump),
`bench-fetch`.

## Data

- `db/migrations/` — schema (applied by the crawler on connect and via `sqlx`).
- `tools/sqlite-to-pg/` — one-shot SQLite→Postgres migration of pre-platform data.
- `crawl_stats_history` grows ~2,880 rows/day (30s tick).

## Docs

- **Crawler**: [`crawler/README.md`](crawler/README.md) — build, run, CLI, flags
- **Architecture**: [`ARCHITECTURE.md`](ARCHITECTURE.md) — source-traced system overview
- **Validation**: [`VALIDATION.md`](VALIDATION.md) — benchmark findings F1–F15 + closing decision
- **Privacy/visibility**: [`docs/PRIVACY.md`](docs/PRIVACY.md)
- **Benchmarks**: [`benchmark/`](benchmark/) — `bench.sh` (windowed report), `liveness.sh` (dashboard), `experiments/` (archived one-offs)
