## Why

The crawler's output today lives only in a SQLite file with no surface to explore it: search is a CLI `LIKE` query, there is zero monitoring beyond rotating logs, and there is no way to inspect crawler health or collected data. Building a web platform (search, admin API, dashboard) requires a shared, concurrent, server-grade store and a full monitoring pipeline — SQLite's single-writer model and lack of fuzzy/vector search cannot support it.

## What Changes

- **BREAKING**: Replace SQLite as the single store with **PostgreSQL**. The crawler storage layer refactors from `rusqlite` to `sqlx` (pure SQL, no ORM) behind the existing 7-method `Storage` interface; `snapshot` (VACUUM INTO) is replaced by `pg_dump`.
- **Migrate all existing data** (8,508 `torrents`, 7,592,072 `scanned` rows) from SQLite to Postgres via a batched COPY tool.
- **Add a monitoring pipeline**: the crawler's 30s `stats_loop` persists a complete snapshot (all ~55 crawl counters + DHT actor diagnostics + jemalloc memory + **system metrics** — tun0 network bandwidth/rates, CPU%, RAM, disk, loadavg) into a `crawl_stats_history` table.
- **Add one Express (TypeScript, strict) API service** with layered routes → controllers → services → repositories (pure SQL), covering:
  - **Search API**: instant fuzzy search over torrent names (`pg_trgm` similarity + GIN), with filters (size, age, file count) and sorting (relevance, newest, largest, name), keyset pagination.
  - **Admin API**: monitoring (latest + history + failures + system metrics) and configuration (`app_config` key/JSONB), plus service health.
- **Add a dashboard** (Vite/React strict-TS, zustand, shadcn/ui, recharts): search page (debounced instant search, filters, sortable results) and monitoring page (live header, time-series charts, system stats, failure breakdowns).
- **No authentication** for now (explicitly deferred).
- **Reserve** pgvector extension + `embeddings` skeleton table for a future classifier/embedder stage (no semantic endpoint yet).
- Compose moves to repo root to orchestrate gluetun, redis, crawler, postgres, api, dashboard.

## Capabilities

### New Capabilities
- `storage/postgres`: PostgreSQL as the single data store — schema (torrents, scanned, crawl_stats_history, app_config), crawler write path via sqlx, and SQLite→Postgres migration.
- `monitoring`: periodic persistence of the full crawler + system metrics snapshot, queryable by the admin API.
- `search`: instant fuzzy torrent search with filters, sorting, and pagination over `pg_trgm`.
- `admin-api`: monitoring/history/failures/system endpoints, configuration management, and health checks.
- `dashboard`: React web UI for searching crawl results and monitoring crawler health.
- `deployment`: root-level docker-compose orchestration of all services with pinned images, healthchecks, log rotation, and non-root containers.

### Modified Capabilities
<!-- none — existing specs live change-local and are archived; no archived spec's behavior changes -->

## Impact

- **Code**: `crawler/src/storage/*` (rusqlite → sqlx), `crawler/src/crawler.rs` (stats_loop persistence), new `crawler/src/sysmetrics/` module, new `db/migrations/`, new `api/` (Express/TS), new `dashboard/` (Vite/React). `crawler/src/main.rs` subcommands (`snapshot`, `bench-fetch`) adapt to Postgres.
- **Dependencies**: add `sqlx` (crawler); Express app deps `pg`, `zod`, `express`, `cors`; dashboard deps `react`, `zustand`, `@tanstack/react-query`, `shadcn/ui`, `recharts`. Remove `rusqlite` from crawler runtime (kept only for the one-shot migration tool if not dropped).
- **Deployment**: root `docker-compose.yml` with `postgres:16-alpine` (tuned `shared_buffers`, autovacuum, `synchronous_commit=off`), pinned `gluetun`, `redis`, crawler (non-root, healthcheck, log rotation), `api`, `dashboard` (static + nginx).
- **Data**: one-time migration of the ~966 MB SQLite DB into Postgres; ongoing `crawl_stats_history` growth (~2,880 rows/day at a 30s tick).
- **Systems**: crawler now hard-depends on Postgres availability (single store; `restart: unless-stopped` + healthcheck ordering).
