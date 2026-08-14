## 1. Data tier (M0 — Postgres infra + migrations)

- [x] 1.1 Move `docker-compose.yml` to repo root; consolidate crawler/redis/gluetun services with pinned images (gluetun, redis 7.4-alpine), log rotation, and existing healthchecks
- [x] 1.2 Add `postgres:16-alpine` service: named volume, healthcheck, tuned config (shared_buffers 512MB, effective_cache_size 4GB, work_mem 16MB, maintenance_work_mem 256MB, max_wal_size 1GB, synchronous_commit=off), statement_timeout
- [x] 1.3 Add `sqlx` (postgres, runtime-tokio) + `sqlx-cli` to the workspace; create `db/migrations/` with the sqlx migration layout
- [x] 1.4 Write migration 0001: create `torrents`, `scanned`, `crawl_stats_history`, `app_config` tables + indexes; enable `pg_trgm` extension + GIN index on `torrents.name`
- [x] 1.5 Write migration 0002: enable `pgvector` extension and create reserved `embeddings` skeleton table
- [x] 1.6 Verify: `docker compose up` brings postgres healthy; migrations are idempotent (`migrate up` twice); schema smoke check via psql

## 2. Data migration (M1)

- [x] 2.1 Add `crawler migrate-sqlite --sqlite <path> --pg <url>` subcommand reading both tables with the existing rusqlite code path
- [x] 2.2 Implement batched COPY (chunked) for `torrents` and `scanned`; preserve all columns; report per-table progress
- [x] 2.3 Implement final count verification (source vs destination per table) with a clear success/failure summary
- [x] 2.4 Verify: run migration on a snapshot; row counts match (8,508 / 7,592,072); integrity spot-checks on sampled rows; trigram index builds

## 3. Crawler storage refactor (M2)

- [x] 3.1 Replace `Storage` internals with an async sqlx `PgPool` keeping the same 7-method interface (insert_batch, scan_status, scan_blocked_batch, record_scanned, search, failure_breakdown, open→connect)
- [x] 3.2 Update all callers (`crawler.rs`, `sampler.rs`, `fetch/mod.rs`, `query.rs`) for async storage; `Storage` becomes connection-pool-based and clonable
- [x] 3.3 Re-point `snapshot` command to Postgres (pg_dump or consistent copy); adapt `bench-fetch` to read from Postgres `scanned`
- [x] 3.4 Remove runtime rusqlite usage (keep only for the migrate-sqlite tool until retired)
- [x] 3.5 Verify: full `cargo test` suite green against Postgres; `cargo clippy` clean; A/B verified/hr unchanged vs SQLite baseline; manual crawl smoke test

## 4. Monitoring pipeline (M3–M4)

- [x] 4.1 Add `sysmetrics` module in the crawler reading tun0 net counters, host meminfo, cgroup memory, cpu delta, disk statfs, loadavg; unit tests with fixture inputs
- [x] 4.2 Extend `stats_loop` to assemble the full snapshot (all crawl counters + DHT diagnostics + jemalloc + instance_nodes JSONB + system metrics)
- [x] 4.3 Persist the snapshot to `crawl_stats_history` every 30s with best-effort error tolerance (crawl continues if insert fails)
- [x] 4.4 Verify: rows appear every 30s with all fields populated; bandwidth tracks tunnel traffic; cpu% sane; failure-tolerant behavior confirmed

## 5. Admin API (M5)

- [x] 5.1 Scaffold `api/` (Express + TS strict, pnpm, zod, pg): config/env validation, pg pool singleton, error middleware, request logger
- [x] 5.2 Implement repositories (pure SQL): stats (latest/history/windowed rates via LAG), failures, system, config (app_config upsert/list/read)
- [x] 5.3 Implement services + controllers: `/api/admin/monitor/latest`, `/api/admin/monitor/history?metric=&range=`, `/api/admin/monitor/failures`, `/api/admin/monitor/system`, `/api/admin/config` (GET/PUT)
- [x] 5.4 Implement `/health` returning per-service status (postgres, redis, crawler, api)
- [x] 5.5 Verify: supertest unit tests (validation, error shapes, config CRUD); curl against live Postgres; invalid inputs return 400

## 6. Search API (M6)

- [x] 6.1 Implement search repository: pg_trgm similarity/word_similarity ranking, filters (size min/max, min file count, min first_seen), sorting (relevance/newest/largest/name), keyset pagination
- [x] 6.2 Implement `GET /api/search` controller with zod validation of all query params; unsupported sort/filters → 400
- [x] 6.3 Verify: unit tests for ranking/filters/sorting/pagination; curl checks; index-backed plan (EXPLAIN) confirmed; <50ms response on current dataset

## 7. Dashboard (M7)

- [x] 7.1 Scaffold `dashboard/` (Vite + React + strict TS + pnpm + tailwind): zustand stores (search, monitor), typed api client, shadcn/ui setup
- [x] 7.2 Build search page: debounced instant search input, filter controls (size, age), sort selector, paginated results table
- [x] 7.3 Build monitoring page: live summary header, time-series charts (verified/hr, unique/hr, routing nodes, memory), system resource charts (network, cpu, memory, disk), failure breakdown
- [x] 7.4 Add vitest component tests for search + monitoring; ensure production build passes type-check and lint
- [x] 7.5 Serve dashboard as a static build via nginx container; wire nginx proxy to the API
- [x] 7.6 Verify: full-stack manual walkthrough (search with filters/sort, monitoring charts live), `vite build` clean, compose up brings dashboard reachable

## 8. Platform integration

- [x] 8.1 Wire root compose: crawler `depends_on` postgres/redis healthy; api `depends_on` postgres; dashboard `depends_on` api
- [x] 8.2 Add `deploy` docs to README (stack layout, ports, healthcheck URLs, API endpoints)
- [x] 8.3 Verify: clean `docker compose up --build` on a fresh checkout starts all services; crawler reaches postgres; dashboard + both API surfaces respond
