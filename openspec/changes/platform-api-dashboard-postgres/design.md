## Context

See `proposal.md` — Why. The platform today is a single Rust crawler persisting to SQLite, with monitoring only in rotating logs. This change adds Postgres as the single store, a monitoring pipeline, one Express API (search + admin), and a React dashboard, orchestrated from the repo root.

## Goals / Non-Goals

**Goals:**
- Replace SQLite with Postgres behind the existing `Storage` interface with zero behavior change to the crawl pipeline.
- Persist a complete 30s snapshot (crawl counters + system metrics) without a separate exporter daemon.
- One Express app, layered (routes → controllers → services → repositories) with pure SQL, no ORM.
- Dashboard: instant fuzzy search + full monitoring UI.
- Deployable from a single root `docker-compose.yml`.

**Non-Goals:**
- No authentication/authorization (explicitly deferred).
- No classifier, LLM, or semantic search endpoint yet — only the pgvector extension + `embeddings` skeleton reserved via migration.
- No metrics-gathering from the Node/React containers themselves beyond service health (system metrics come from the crawler's netns view).
- No horizontal sharding; single Postgres instance.

## Decisions

### D1 — sqlx (async) over tokio-postgres
Crawler uses `sqlx` with the `postgres` + `runtime-tokio` features, query macros where practical but no ORM layers.
- *Rationale:* async-native (the crawler is tokio), built-in connection pooling (`PgPool`), compile-time-checked queries via `query!`, and a first-class migration CLI. tokio-postgres is lower-level and would need a separate pool crate (bb8).
- *Alternatives:* `tokio-postgres` + `bb8` — rejected: more assembly, no migration tooling.

### D2 — System metrics read from the crawler's netns
The crawler already lives inside gluetun's network namespace and already owns the 30s tick. A `sysmetrics` module reads: `tun0` counters (`/proc/net/dev`), host memory (`/proc/meminfo`), container cgroup v2 memory (`/sys/fs/cgroup/memory.current`), CPU delta (`/proc/stat`), disk (`statfs` on `/data`), loadavg (`/proc/loadavg`).
- *Rationale:* no extra exporter container, no Node-side proc parsing, one writer = DRY. tun0 is only visible in gluetun's netns, so this is the only correct vantage point for tunnel bandwidth.
- *Alternatives:* a dedicated metrics sidecar — rejected: extra moving part for data the crawler already times.

### D3 — Typed columns for `crawl_stats_history`
Each metric is a typed column (not a single JSONB blob), so SQL aggregation/window functions are natural and indexes are possible. `instance_nodes` (per-instance breakdown) is JSONB since it is a display structure.
- *Rationale:* windowed rate math (`LAG()`) needs real columns; JSONB would force parse-at-query-time and lose type safety.
- *Trade-off:* wide table (~70 columns); acceptable for a 30s-tick monitoring table.

### D4 — One Express app, layered, TypeScript strict
`api/` is a single service (search + admin routers share the same pool and repositories). Layout: `config/` (zod env + `pg` pool singleton), `routes/`, `controllers/`, `services/`, `repositories/`, `sql/` (shared query fragments), `middleware/`, `types/`, `utils/`.
- *Rationale:* one app avoids duplicating pool/config/health wiring; the layered split keeps files small (no god files) and makes each layer unit-testable.
- *Alternatives:* separate search/admin apps — rejected: same DB, same config, same infra; splitting adds a second health surface for no isolation gain at this scale.

### D5 — Fuzzy search via `pg_trgm` GIN + similarity ranking
Search uses `word_similarity`/`similarity` with a GIN trigram index on `torrents.name`; filters/sorting append to one parameterized query; keyset pagination via cursor on `(similarity, info_hash)` or `(first_seen, info_hash)`.
- *Rationale:* instant (index-backed), typo-tolerant, native Postgres — no extra search engine (Typesense/Meilisearch) to operate. Semantic search is deferred to the classifier stage (pgvector reserved).
- *Trade-off:* trigram similarity is heuristic, not semantic; acceptable now, pgvector later.

### D6 — Postgres tuning profile
Pinned `postgres:16-alpine` with `shared_buffers=512MB`, `effective_cache_size=4GB`, `work_mem=16MB`, `maintenance_work_mem=256MB`, `synchronous_commit=off`, `max_wal_size=1GB`, aggressive autovacuum on `scanned`, `statement_timeout` for API queries.
- *Rationale:* crawler index data is regenerable (torrents/scanned can be re-crawled), so relaxing durability for write throughput is a sound trade; the host has ~4 GB free RAM on top of the crawler's ~230 MB.
- *Trade-off:* `synchronous_commit=off` risks losing the last ~0.5s of commits on a crash — acceptable for a re-crawlable index.

### D7 — Migration tool as a crawler subcommand
A one-shot `crawler migrate-sqlite --sqlite <path> --pg <url>` streams both tables in chunks using COPY, then prints count verification.
- *Rationale:* reuses the crawler's rusqlite code path (dropped only after migration), keeps the tool in-repo, and is auditable. `snapshot`/`bench-fetch` are re-pointed at Postgres.
- *Trade-off:* the crawler keeps a rusqlite dev-dependency solely for the migrator until it's retired.

## Risks / Trade-offs

- **Postgres availability is a hard dependency for the crawler** → `restart: unless-stopped` + healthcheck ordering + `depends_on: condition: service_healthy`; stats persistence is already failure-tolerant (monitoring spec: crawl continues if a stats write fails).
- **Crawler storage refactor could perturb the hot path** (scan_status / scan_blocked_batch per sampled hash) → keep the same SQL shapes, add a statement timeout only to admin queries, not crawler reads; A/B verified/hr before/after as a gate.
- **Migration of 7.6M rows could take minutes and load the DB** → run COPY in bounded batches with a final count check; run during low-traffic window; crawler keeps writing to SQLite until cutover, then flips.
- **Wide history table grows** (~2,880 rows/day ≈ tiny) → no retention needed for now; add a retention migration later if it ever matters.
- **trigram index build on `name` is fast on 8k rows** → index build included in migration; no concern at current scale.
- **Dashboard/API introduce a second language to the repo** → locked to strict TS + pnpm; shared `types/` for the API contract to keep the client typed.

## Migration Plan

1. **M0**: add Postgres service + `db/migrations/`; migrate schema up on empty DB; `docker compose` moves to root.
2. **M1**: run `crawler migrate-sqlite` to copy SQLite → Postgres; verify counts + integrity spot-checks.
3. **M2**: refactor crawler storage to sqlx against Postgres; cut over crawler to Postgres; remove SQLite runtime usage; A/B verified/hr.
4. **M3–M4**: stats + system-metrics persistence.
5. **M5–M6**: admin + search API.
6. **M7**: dashboard.
7. **Rollback**: keep the pre-cutover SQLite file untouched until the crawler has run stably on Postgres for a day; rollback = restore SQLite path + old compose.

## Open Questions

None — deferred items (auth, classifier, embeddings endpoint) are explicit non-goals, not open decisions.
