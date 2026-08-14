## Purpose

PostgreSQL becomes the single persistent data store for the crawler platform: schema ownership, the crawler's read/write path, and the one-time migration from SQLite.

## ADDED Requirements

### Requirement: PostgreSQL is the single store
The crawler SHALL read and write all persistent data through PostgreSQL, with no SQLite file used at runtime.

#### Scenario: Crawler starts against Postgres
- **WHEN** the crawler process starts with a Postgres connection
- **THEN** it connects to Postgres, applies migrations, and begins sampling with no SQLite file involved

#### Scenario: Runtime write path
- **WHEN** a torrent is verified or a scan is recorded
- **THEN** the record is upserted into the Postgres `torrents` or `scanned` table within the existing batching behavior

### Requirement: Migrations own the schema
All schema changes SHALL be applied via versioned SQL migrations under `db/migrations/`, and migrations SHALL be idempotent and runnable on an empty database.

#### Scenario: Fresh database initializes
- **WHEN** a new Postgres database is provisioned
- **THEN** running the migrations creates `torrents`, `scanned`, `crawl_stats_history`, and `app_config` with their indexes

#### Scenario: Re-run is a no-op
- **WHEN** migrations are applied to an already-migrated database
- **THEN** no duplicate objects are created and no error is raised

### Requirement: Existing data migrates losslessly
The migration tool SHALL copy all rows from the SQLite `torrents` and `scanned` tables into Postgres, preserving every field, and SHALL report a final count comparison.

#### Scenario: Row counts match after migration
- **WHEN** migration completes
- **THEN** `torrents` and `scanned` row counts in Postgres equal the SQLite source counts

#### Scenario: Batched large-table copy
- **WHEN** migrating the `scanned` table (millions of rows)
- **THEN** rows are copied in bounded batches (e.g. via COPY with chunking) so memory stays flat

### Requirement: Crawler storage interface preserved
The crawler's `Storage` interface (batched upsert, scan-status check, batched blocked check, record-scanned, search, failure breakdown) SHALL keep its existing semantics after the Postgres refactor, with no behavior change for the sampling/fetch pipeline.

#### Scenario: Existing Rust suite passes
- **WHEN** the crawler test suite runs against Postgres
- **THEN** all storage and pipeline tests pass unchanged

#### Scenario: Verified/hr unchanged
- **WHEN** the crawler runs against Postgres under the production config
- **THEN** the verified/hr steady state matches the SQLite baseline within measurement noise

### Requirement: Offline tooling operates on Postgres
The `snapshot` and `bench-fetch` commands SHALL work against the Postgres store (snapshot via `pg_dump` or equivalent consistent copy).

#### Scenario: Snapshot produces a consistent copy
- **WHEN** the snapshot command runs against a live crawler DB
- **THEN** it produces a consistent, standalone Postgres dump that can be restored

#### Scenario: Bench-fetch replays from Postgres
- **WHEN** `bench-fetch` samples a class of hashes
- **THEN** it reads outcome data from the Postgres `scanned` table as it did from SQLite
