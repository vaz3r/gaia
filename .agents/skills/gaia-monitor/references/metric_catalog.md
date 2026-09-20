# GAIA Metrics & Telemetry Catalog

This catalog documents the key metrics collected by the `gaia-monitor` skill, including their telemetry sources, formulas, and operational relevance.

---

## 1. Ingestion & Crawler Metrics

| Metric | Source | Formula / Query | Operational Significance |
| :--- | :--- | :--- | :--- |
| **`torrents_total`** | PostgreSQL `torrents` | `SELECT count(*) FROM torrents;` | Cumulative verified unique torrents in the ecosystem. |
| **`verified_1h`** | PostgreSQL `torrents` | `SELECT count(*) FROM torrents WHERE verified_at > now() - interval '1 hour';` | Hourly ingestion velocity (torrents/hr). Primary health indicator of DHT ingestion. |
| **`verified_24h`** | PostgreSQL `torrents` | `SELECT count(*) FROM torrents WHERE verified_at > now() - interval '24 hours';` | Daily rolling ingestion baseline. |
| **`hourly_moving_avg`** | PostgreSQL `torrents` | `verified_24h / 24.0` | Baseline against which sudden throughput drops (> 30%) are detected. |
| **`bep9_success_rate`** | PostgreSQL `metrics` | `unique_infohashes / fetch_attempts * 100` | Percentage of attempted peer handshakes that successfully yield metadata. |
| **`dht_query_volume`** | PostgreSQL `metrics` | `inbound_get_peers + inbound_announce_peer` | Active query load placed on the crawler node by external DHT nodes. |
| **`outbound_timeout_rate`**| PostgreSQL `metrics` | `outbound_timeouts / outbound_queries * 100` | Network packet drop / peer unreachability rate. |
| **`routing_table_len`** | PostgreSQL `metrics` | `SELECT metric_value FROM metrics WHERE metric_name='routing_table_len'` | Total active nodes in crawler Kademlia routing table. Should remain > 5,000. |

---

## 2. Classification & Machine Learning Metrics

| Metric | Source | Formula / Query | Operational Significance |
| :--- | :--- | :--- | :--- |
| **`unclassified_backlog`** | PostgreSQL `torrents` | `SELECT count(*) FROM torrents WHERE category IS NULL;` | Queue depth of torrents awaiting category inference. |
| **`batch_throughput`** | `gaia-classifier` logs | Extracted from `Batch: N \| Rate: X rec/s` | Raw processing speed of LightGBM model. |
| **`timing_fetch`** | `gaia-classifier` logs | Extracted `fetch Xs` | Time spent selecting unclassified records from PostgreSQL. |
| **`timing_infer`** | `gaia-classifier` logs | Extracted `infer Xs` | Time spent on regex tokenization and LightGBM CPU inference. |
| **`timing_db_commit`** | `gaia-classifier` logs | Extracted `db Xs` | Time spent executing `UPDATE torrents` in PostgreSQL. High values indicate write lock contention. |
| **`category_distribution`**| PostgreSQL `torrents` | `SELECT category, count(*) FROM torrents GROUP BY category` | Proportions of ingested content (Movies, TV, Music, Anime, etc.). |
| **`ml_anomaly_score`** | `gaia-ml` logs | Extracted `Window: ... \| Score: X.XXX` | Isolation Forest anomaly score. Values > 0.60 indicate abnormal traffic/swarm behavior. |
| **`scoring_throughput`** | `gaia-ml` logs | Extracted `Scored N torrents` / min | Velocity of the risk and quality scoring pipeline. |

---

## 3. Storage, Database & Search Replication Metrics

| Metric | Source | Formula / Query | Operational Significance |
| :--- | :--- | :--- | :--- |
| **`pg_cache_hit_ratio`** | PostgreSQL `pg_stat_database` | `sum(blks_hit) * 100.0 / sum(blks_hit + blks_read)` | Buffer cache efficiency. Must remain > 99.0% for optimal performance. |
| **`pg_active_connections`**| PostgreSQL `pg_stat_activity` | `SELECT count(*) FROM pg_stat_activity WHERE state = 'active';` | Concurrent active worker queries. |
| **`pg_db_size`** | PostgreSQL | `SELECT pg_size_pretty(pg_database_size('craw'));` | Physical disk footprint of master database. |
| **`pg_slow_queries`** | PostgreSQL `pg_stat_activity` | `WHERE state = 'active' AND (now() - query_start) > interval '5 seconds'` | Queries causing pipeline bottlenecks or transaction lock wait. |
| **`pgbouncer_client_conn`**| PgBouncer `SHOW CLIENTS` | Active connected clients vs `max_client_conn` (500). |
| **`meili_indexed_docs`** | Meilisearch `/indexes/torrents` | `numberOfDocuments` from `/stats` | Count of searchable documents in Meilisearch. |
| **`meili_sync_lag`** | Postgres vs Meilisearch | `total_torrents - meili_indexed_docs` | Unsynchronized documents pending CDC sync. |
| **`meili_last_rebuild_age`**| `gaia-portal-sync` logs | Minutes elapsed since last `REBUILD & ATOMIC SWAP`. | Should not exceed 150 minutes (standard rebuild interval is 120m). |
| **`redis_used_memory`** | Redis `INFO memory` | `used_memory_human` | Cache memory footprint vs 1024mb limit. |
| **`redis_evicted_keys`** | Redis `INFO stats` | `evicted_keys` | Keys purged due to memory pressure. |

---

## 4. Host, Container Fleet & Network Metrics

| Metric | Source | Operational Target |
| :--- | :--- | :--- |
| **`host_load_1m/5m/15m`** | `/proc/loadavg` | Warning if > 12.0, Critical if > 16.0. |
| **`host_ram_available`** | `free -m` | Warning if < 1.0GB, Critical if < 500MB. |
| **`host_disk_usage_pct`** | `df -hP /` & `/home/core/gaia-data` | Warning if > 80%, Critical if > 90%. |
| **`container_restart_count`**| `docker inspect .RestartCount` | Should be 0 for stable running containers. |
| **`container_health_state`**| `docker inspect .State.Health.Status` | Must be `healthy` for containers with healthchecks. |
| **`wireguard_handshake_age`**| `wg show wg0 latest-handshakes` | Warning if > 180s, Critical if > 300s. |
| **`tunnel_ping_latency`** | `ping -c 2 10.99.0.1` | Must be < 100ms with 0% packet loss. |
| **`opsec_killswitch_active`**| `iptables -S OUTPUT` | Port 80/443 must be REJECTED for non-gateway IPs. |
| **`dot_active`** | `resolvectl status` | DNS-over-TLS (`+DNSOverTLS`) must be enabled. |
