-- 0001_init.sql — base schema for the crawler platform.
-- Run once; applied idempotently by sqlx migrations.

-- Fuzzy search over torrent names (trigram similarity).
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE torrents (
    info_hash  BYTEA PRIMARY KEY,
    name       TEXT NOT NULL,
    size_bytes BIGINT,
    file_count BIGINT,
    first_seen BIGINT NOT NULL,
    last_seen  BIGINT NOT NULL
);

-- Trigram GIN index for instant fuzzy search over release names.
CREATE INDEX torrents_name_trgm_idx ON torrents USING GIN (name gin_trgm_ops);

CREATE TABLE scanned (
    info_hash      BYTEA PRIMARY KEY,
    status         TEXT NOT NULL CHECK (status IN ('ok', 'skipped', 'failed')),
    info_bytes     BYTEA,
    raw_name       TEXT,
    attempts       BIGINT NOT NULL DEFAULT 0,
    last_attempt   BIGINT NOT NULL,
    next_attempt   BIGINT NOT NULL,
    failure_reason TEXT
);

-- The `scanned` table is update-heavy (attempt counts), so the autovacuum
-- settings in compose keep dead tuples bounded; the PK index covers the
-- per-hash admission checks.

CREATE TABLE crawl_stats_history (
    ts                           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Crawl counters (cumulative since process start)
    hashes_sampled               BIGINT NOT NULL DEFAULT 0,
    hashes_unique                BIGINT NOT NULL DEFAULT 0,
    hashes_announced             BIGINT NOT NULL DEFAULT 0,
    announces_deduped_redis      BIGINT NOT NULL DEFAULT 0,
    announces_emitted            BIGINT NOT NULL DEFAULT 0,
    shadow_emitted               BIGINT NOT NULL DEFAULT 0,
    shadow_filtered              BIGINT NOT NULL DEFAULT 0,
    shadow_near_miss_1           BIGINT NOT NULL DEFAULT 0,
    shadow_near_miss_2           BIGINT NOT NULL DEFAULT 0,
    shadow_near_miss_1_sparse    BIGINT NOT NULL DEFAULT 0,
    shadow_near_miss_1_stalled   BIGINT NOT NULL DEFAULT 0,
    liveness_sweeps              BIGINT NOT NULL DEFAULT 0,
    fetches_attempted            BIGINT NOT NULL DEFAULT 0,
    fetches_failed               BIGINT NOT NULL DEFAULT 0,
    metadata_verified            BIGINT NOT NULL DEFAULT 0,
    records_persisted            BIGINT NOT NULL DEFAULT 0,
    terminal_dead                BIGINT NOT NULL DEFAULT 0,
    -- Pipeline depth (snapshots)
    fetch_in_flight              BIGINT NOT NULL DEFAULT 0,
    queue_depth                  BIGINT NOT NULL DEFAULT 0,
    -- Per-peer failure taxonomy
    connect_timeout              BIGINT NOT NULL DEFAULT 0,
    connect_refused              BIGINT NOT NULL DEFAULT 0,
    connection_reset             BIGINT NOT NULL DEFAULT 0,
    connection_closed            BIGINT NOT NULL DEFAULT 0,
    no_bep10                     BIGINT NOT NULL DEFAULT 0,
    no_ut_metadata               BIGINT NOT NULL DEFAULT 0,
    metadata_rejected            BIGINT NOT NULL DEFAULT 0,
    parse_error                  BIGINT NOT NULL DEFAULT 0,
    sha1_mismatch                BIGINT NOT NULL DEFAULT 0,
    empty_peers                  BIGINT NOT NULL DEFAULT 0,
    fetch_deadline               BIGINT NOT NULL DEFAULT 0,
    early_abort                  BIGINT NOT NULL DEFAULT 0,
    peer_errors_other            BIGINT NOT NULL DEFAULT 0,
    -- Verified torrents split by discovery source
    verified_announced           BIGINT NOT NULL DEFAULT 0,
    verified_sampled             BIGINT NOT NULL DEFAULT 0,
    verified_lookedup            BIGINT NOT NULL DEFAULT 0,
    verified_tracker             BIGINT NOT NULL DEFAULT 0,
    -- BEP 33 scrape shadow
    scrape_saw_seeds             BIGINT NOT NULL DEFAULT 0,
    verified_with_seeds          BIGINT NOT NULL DEFAULT 0,
    verified_without_seeds       BIGINT NOT NULL DEFAULT 0,
    failed_with_seeds            BIGINT NOT NULL DEFAULT 0,
    failed_without_seeds         BIGINT NOT NULL DEFAULT 0,
    -- Liveness / discriminator
    discriminator_filtered       BIGINT NOT NULL DEFAULT 0,
    lookups_emitted              BIGINT NOT NULL DEFAULT 0,
    lookups_deduped_redis        BIGINT NOT NULL DEFAULT 0,
    -- DHT actor diagnostics (primary instance)
    routing_nodes                BIGINT NOT NULL DEFAULT 0,
    announced_hashes             BIGINT NOT NULL DEFAULT 0,
    active_lookups               BIGINT NOT NULL DEFAULT 0,
    announce_tokens              BIGINT NOT NULL DEFAULT 0,
    pending_queries              BIGINT NOT NULL DEFAULT 0,
    announces_received           BIGINT NOT NULL DEFAULT 0,
    announces_token_rejected     BIGINT NOT NULL DEFAULT 0,
    announces_suppressed_readonly BIGINT NOT NULL DEFAULT 0,
    lookups_received             BIGINT NOT NULL DEFAULT 0,
    -- Per-instance routing table / query totals (display structure)
    instance_nodes               JSONB,
    -- Derived rate (extrapolated unique hashes per hour)
    unique_per_hr                DOUBLE PRECISION,
    -- Allocator state (jemalloc), MB
    jemalloc_allocated           DOUBLE PRECISION,
    jemalloc_active              DOUBLE PRECISION,
    jemalloc_mapped              DOUBLE PRECISION,
    jemalloc_retained            DOUBLE PRECISION,
    -- System resource metrics
    net_rx_bytes                 BIGINT NOT NULL DEFAULT 0,
    net_tx_bytes                 BIGINT NOT NULL DEFAULT 0,
    net_rx_rate_bps              DOUBLE PRECISION,
    net_tx_rate_bps              DOUBLE PRECISION,
    host_mem_total               BIGINT NOT NULL DEFAULT 0,
    host_mem_available           BIGINT NOT NULL DEFAULT 0,
    container_mem_current        BIGINT NOT NULL DEFAULT 0,
    cpu_percent                  DOUBLE PRECISION,
    disk_total_bytes             BIGINT NOT NULL DEFAULT 0,
    disk_free_bytes              BIGINT NOT NULL DEFAULT 0,
    loadavg_1                    DOUBLE PRECISION,
    loadavg_5                    DOUBLE PRECISION,
    loadavg_15                   DOUBLE PRECISION
);

-- Monitoring reads always filter by time range, ordered ascending.
CREATE INDEX crawl_stats_history_ts_idx ON crawl_stats_history (ts);

CREATE TABLE app_config (
    key        TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
