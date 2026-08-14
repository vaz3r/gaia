pub mod model;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

pub use model::{
    backoff_secs, EMPTY_PEERS_RETRY_SECS, ScannedRecord, ScannedStatus, TorrentRecord,
};

/// PostgreSQL-backed storage. `sqlx::migrate!` applies `db/migrations` at
/// connect time, so the schema is always current. A single pooled connection
/// serves the crawler's reads and writes.
#[derive(Clone)]
pub struct Storage {
    pool: PgPool,
}

impl Storage {
    /// Connect to Postgres and apply pending migrations.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(url)
            .await
            .with_context(|| format!("connect postgres {url}"))?;
        crate::db::migrate(&pool).await.context("apply migrations")?;
        Ok(Self { pool })
    }

    /// Insert or update a batch of records in a single transaction, preserving
    /// the original `first_seen` on duplicates.
    pub async fn insert_batch(&self, records: &[TorrentRecord]) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin insert tx")?;
        for r in records {
            sqlx::query(
                "INSERT INTO torrents (info_hash, name, size_bytes, file_count, first_seen, last_seen)
                 VALUES ($1::bytea, $2, $3, $4, $5, $6)
                 ON CONFLICT (info_hash) DO UPDATE SET
                     name       = excluded.name,
                     size_bytes = excluded.size_bytes,
                     file_count = excluded.file_count,
                     last_seen  = excluded.last_seen",
            )
            .bind(r.info_hash.as_slice())
            .bind(r.name.as_str())
            .bind(r.size_bytes)
            .bind(r.file_count)
            .bind(r.first_seen)
            .bind(r.last_seen)
            .execute(&mut *tx)
            .await
            .context("upsert torrent row")?;
        }
        tx.commit().await.context("commit insert tx")?;
        Ok(())
    }

    /// The recorded scan status for `info_hash`, or `None` if never attempted.
    pub async fn scan_status(&self, info_hash: &[u8; 20]) -> Result<Option<ScannedStatus>> {
        let row = sqlx::query(
            "SELECT status, attempts, next_attempt, failure_reason FROM scanned WHERE info_hash = $1::bytea",
        )
        .bind(info_hash.as_slice())
        .fetch_optional(&self.pool)
        .await
        .context("query scan status")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let status: String = row.get(0);
        let attempts: i64 = row.get(1);
        let next_attempt: i64 = row.get(2);
        let failure_reason: Option<String> = row.get(3);
        Ok(Some(match status.as_str() {
            "ok" => ScannedStatus::Ok,
            "skipped" => ScannedStatus::Skipped,
            _ => ScannedStatus::Failed {
                attempts,
                next_attempt,
                failure_reason,
            },
        }))
    }

    /// True if `info_hash` should not be fetched right now: already accepted,
    /// already filtered out, or a recent failure still inside its backoff
    /// window. `now` is a unix timestamp in seconds.
    ///
    /// Batched production admission uses `scan_blocked_batch`; this helper
    /// keeps the single-hash check available to tests.
    #[cfg(test)]
    pub async fn scan_blocked(&self, info_hash: &[u8; 20], now: i64) -> Result<bool> {
        Ok(self
            .scan_blocked_batch(&[*info_hash], now)
            .await?
            .contains(info_hash))
    }

    /// Batched `scan_blocked`: returns the subset of `info_hashes` that should
    /// NOT be fetched right now (accepted, filtered, or inside a backoff
    /// window). One `IN` query instead of N point lookups keeps pipeline
    /// admission cheap as the unique-hash stream grows. Hashes absent from the
    /// `scanned` table are never blocked.
    pub async fn scan_blocked_batch(
        &self,
        info_hashes: &[[u8; 20]],
        now: i64,
    ) -> Result<Vec<[u8; 20]>> {
        if info_hashes.is_empty() {
            return Ok(Vec::new());
        }
        // Cap the IN-list at Postgres' bind limit by chunking; 64-entry chunks
        // are far below it but keep the query bounded.
        let mut blocked = Vec::new();
        for chunk in info_hashes.chunks(64) {
            let placeholders: Vec<String> = (1..=chunk.len())
                .map(|i| format!("${i}::bytea"))
                .collect();
            let sql = format!(
                "SELECT info_hash, status, next_attempt FROM scanned WHERE info_hash IN ({})",
                placeholders.join(",")
            );
            let mut query = sqlx::query(&sql);
            for h in chunk {
                query = query.bind(h.as_slice());
            }
            let rows = query.fetch_all(&self.pool).await.context("batched scan check")?;
            for row in rows {
                let hash_bytes: Vec<u8> = row.get(0);
                if hash_bytes.len() != 20 {
                    continue;
                }
                let mut h = [0u8; 20];
                h.copy_from_slice(&hash_bytes);
                let status: String = row.get(1);
                let next_attempt: i64 = row.get(2);
                let is_blocked = match status.as_str() {
                    "ok" | "skipped" => true,
                    _ => next_attempt > now,
                };
                if is_blocked {
                    blocked.push(h);
                }
            }
        }
        Ok(blocked)
    }

    /// Upsert a `scanned` row. A `Failed` record increments the attempt count
    /// and schedules the next attempt with exponential backoff.
    pub async fn record_scanned(&self, rec: &ScannedRecord) -> Result<()> {
        match &rec.status {
            ScannedStatus::Ok | ScannedStatus::Skipped => {
                let status = match rec.status {
                    ScannedStatus::Ok => "ok",
                    _ => "skipped",
                };
                sqlx::query(
                    "INSERT INTO scanned (info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt)
                     VALUES ($1::bytea, $2, $3::bytea, $4, 1, $5, $5)
                     ON CONFLICT (info_hash) DO UPDATE SET
                         status = excluded.status,
                         info_bytes = excluded.info_bytes,
                         raw_name = excluded.raw_name,
                         attempts = scanned.attempts + 1,
                         last_attempt = excluded.last_attempt,
                         next_attempt = excluded.last_attempt",
                )
                .bind(rec.info_hash.as_slice())
                .bind(status)
                .bind(rec.info_bytes.as_deref())
                .bind(rec.raw_name.as_deref())
                .bind(rec.last_attempt)
                .execute(&self.pool)
                .await
                .context("upsert ok/skipped scanned row")?;
            }
            ScannedStatus::Failed { attempts, next_attempt, failure_reason } => {
                sqlx::query(
                    "INSERT INTO scanned (info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt, failure_reason)
                     VALUES ($1::bytea, 'failed', NULL, NULL, $2, $3, $4, $5)
                     ON CONFLICT (info_hash) DO UPDATE SET
                         status = 'failed',
                         attempts = $2,
                         last_attempt = $3,
                         next_attempt = $4,
                         failure_reason = $5",
                )
                .bind(rec.info_hash.as_slice())
                .bind(attempts)
                .bind(rec.last_attempt)
                .bind(next_attempt)
                .bind(failure_reason.as_deref())
                .execute(&self.pool)
                .await
                .context("upsert failed scanned row")?;
            }
        }
        Ok(())
    }

    /// Case-insensitive substring search over the raw release name.
    pub async fn search(&self, query: &str) -> Result<Vec<TorrentRecord>> {
        let rows = sqlx::query(
            "SELECT info_hash, name, size_bytes, file_count, first_seen, last_seen
             FROM torrents
             WHERE name ILIKE '%' || $1 || '%' ESCAPE '\\'
             ORDER BY last_seen DESC
             LIMIT 200",
        )
        .bind(query.replace('%', r"\%").replace('_', r"\_"))
        .fetch_all(&self.pool)
        .await
        .context("search torrents")?;
        Ok(rows.iter().map(row_to_record).collect())
    }

    /// Aggregate metadata fetch failures by dominant reason from the `scanned`
    /// table. Returns `(reason, count)` sorted descending by count.
    pub async fn failure_breakdown(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT COALESCE(failure_reason, 'unknown'), COUNT(*) AS n
             FROM scanned WHERE status = 'failed'
             GROUP BY COALESCE(failure_reason, 'unknown')
             ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("failure breakdown")?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>(0), r.get::<i64, _>(1)))
            .collect())
    }

    /// Best-effort insert of one full monitoring snapshot. Failures are logged
    /// and swallowed so the crawl loop never breaks on a stats write.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_crawl_stats(
        &self,
        s: &crate::stats::CrawlSnapshot,
        instance_nodes: &serde_json::Value,
    ) {
        let result = sqlx::query(
            "INSERT INTO crawl_stats_history (
                hashes_sampled, hashes_unique, hashes_announced, announces_deduped_redis,
                announces_emitted, shadow_emitted, shadow_filtered, shadow_near_miss_1,
                shadow_near_miss_2, shadow_near_miss_1_sparse, shadow_near_miss_1_stalled,
                liveness_sweeps, fetches_attempted, fetches_failed, metadata_verified,
                records_persisted, terminal_dead, fetch_in_flight, queue_depth,
                connect_timeout, connect_refused, connection_reset, connection_closed,
                no_bep10, no_ut_metadata, metadata_rejected, parse_error, sha1_mismatch,
                empty_peers, fetch_deadline, early_abort, peer_errors_other,
                verified_announced, verified_sampled, verified_lookedup, verified_tracker,
                scrape_saw_seeds, verified_with_seeds, verified_without_seeds,
                failed_with_seeds, failed_without_seeds, discriminator_filtered,
                lookups_emitted, lookups_deduped_redis,
                routing_nodes, announced_hashes, active_lookups, announce_tokens,
                pending_queries, announces_received, announces_token_rejected,
                announces_suppressed_readonly, lookups_received,
                instance_nodes, unique_per_hr,
                jemalloc_allocated, jemalloc_active, jemalloc_mapped, jemalloc_retained,
                net_rx_bytes, net_tx_bytes, net_rx_rate_bps, net_tx_rate_bps,
                host_mem_total, host_mem_available, container_mem_current, cpu_percent,
                disk_total_bytes, disk_free_bytes, loadavg_1, loadavg_5, loadavg_15
            ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
                $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,
                $37,$38,$39,$40,$41,$42,$43,$44,$45,$46,$47,$48,$49,$50,$51,$52,$53,
                $54,$55,$56,$57,$58,$59,$60,$61,$62,$63,$64,$65,$66,$67,$68,$69,$70,$71,$72
            )",
        )
        .bind(s.hashes_sampled as i64)
        .bind(s.hashes_unique as i64)
        .bind(s.hashes_announced as i64)
        .bind(s.announces_deduped_redis as i64)
        .bind(s.announces_emitted as i64)
        .bind(s.shadow_emitted as i64)
        .bind(s.shadow_filtered as i64)
        .bind(s.shadow_near_miss_1 as i64)
        .bind(s.shadow_near_miss_2 as i64)
        .bind(s.shadow_near_miss_1_sparse as i64)
        .bind(s.shadow_near_miss_1_stalled as i64)
        .bind(s.liveness_sweeps as i64)
        .bind(s.fetches_attempted as i64)
        .bind(s.fetches_failed as i64)
        .bind(s.metadata_verified as i64)
        .bind(s.records_persisted as i64)
        .bind(s.terminal_dead as i64)
        .bind(s.fetch_in_flight as i64)
        .bind(s.queue_depth as i64)
        .bind(s.connect_timeout as i64)
        .bind(s.connect_refused as i64)
        .bind(s.connection_reset as i64)
        .bind(s.connection_closed as i64)
        .bind(s.no_bep10 as i64)
        .bind(s.no_ut_metadata as i64)
        .bind(s.metadata_rejected as i64)
        .bind(s.parse_error as i64)
        .bind(s.sha1_mismatch as i64)
        .bind(s.empty_peers as i64)
        .bind(s.fetch_deadline as i64)
        .bind(s.early_abort as i64)
        .bind(s.peer_errors_other as i64)
        .bind(s.verified_announced as i64)
        .bind(s.verified_sampled as i64)
        .bind(s.verified_lookedup as i64)
        .bind(s.verified_tracker as i64)
        .bind(s.scrape_saw_seeds as i64)
        .bind(s.verified_with_seeds as i64)
        .bind(s.verified_without_seeds as i64)
        .bind(s.failed_with_seeds as i64)
        .bind(s.failed_without_seeds as i64)
        .bind(s.discriminator_filtered as i64)
        .bind(s.lookups_emitted as i64)
        .bind(s.lookups_deduped_redis as i64)
        .bind(s.routing_nodes as i64)
        .bind(s.announced_hashes as i64)
        .bind(s.active_lookups as i64)
        .bind(s.announce_tokens as i64)
        .bind(s.pending_queries as i64)
        .bind(s.announces_received as i64)
        .bind(s.announces_token_rejected as i64)
        .bind(s.announces_suppressed_readonly as i64)
        .bind(s.lookups_received as i64)
        .bind(instance_nodes)
        .bind(s.unique_per_hr)
        .bind(s.jemalloc_allocated)
        .bind(s.jemalloc_active)
        .bind(s.jemalloc_mapped)
        .bind(s.jemalloc_retained)
        .bind(s.net_rx_bytes as i64)
        .bind(s.net_tx_bytes as i64)
        .bind(s.net_rx_rate_bps)
        .bind(s.net_tx_rate_bps)
        .bind(s.host_mem_total as i64)
        .bind(s.host_mem_available as i64)
        .bind(s.container_mem_current as i64)
        .bind(s.cpu_percent)
        .bind(s.disk_total_bytes as i64)
        .bind(s.disk_free_bytes as i64)
        .bind(s.loadavg_1)
        .bind(s.loadavg_5)
        .bind(s.loadavg_15)
        .execute(&self.pool)
        .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "crawl stats persistence failed (continuing)");
        }
    }
}

fn row_to_record(row: &sqlx::postgres::PgRow) -> TorrentRecord {
    let hash_bytes: Vec<u8> = row.get(0);
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&hash_bytes);
    TorrentRecord {
        info_hash,
        name: row.get(1),
        size_bytes: row.get(2),
        file_count: row.get(3),
        first_seen: row.get(4),
        last_seen: row.get(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Postgres URL for tests; override via GAIA_TEST_PG.
    fn test_pg() -> String {
        std::env::var("GAIA_TEST_PG").unwrap_or_else(|_| {
            "postgres://crawler:crawler@127.0.0.1:5432/crawler".to_string()
        })
    }

    /// Wipe both tables so a test starts from a clean state regardless of
    /// prior runs or test ordering within a shared test database.
    async fn clean(db: &Storage) {
        sqlx::query("TRUNCATE TABLE torrents, scanned, embeddings")
            .execute(&db.pool)
            .await
            .unwrap();
    }

    fn record(hash: u8, first: i64, last: i64) -> TorrentRecord {
        TorrentRecord {
            info_hash: [hash; 20],
            name: "The Matrix 1999".into(),
            size_bytes: Some(1024),
            file_count: Some(1),
            first_seen: first,
            last_seen: last,
        }
    }

    #[tokio::test]
    async fn upsert_preserves_first_seen() {
        let db = Storage::connect(&test_pg()).await.unwrap();
        clean(&db).await;
        db.insert_batch(&[record(1, 100, 100)]).await.unwrap();
        db.insert_batch(&[record(1, 9999, 200)]).await.unwrap();

        let rows = db.search("matrix").await.unwrap();
        assert!(rows.iter().any(|r| r.first_seen == 100), "first_seen immutable");
        assert!(rows.iter().any(|r| r.last_seen == 200), "last_seen refreshed");
    }

    #[tokio::test]
    async fn search_match_and_no_match() {
        let db = Storage::connect(&test_pg()).await.unwrap();
        clean(&db).await;
        db.insert_batch(&[record(3, 1, 1)]).await.unwrap();

        let hits = db.search("maTrIx").await.unwrap();
        assert!(!hits.is_empty(), "search must be case-insensitive");

        let misses = db.search("nonexistent").await.unwrap();
        assert!(misses.is_empty());
    }

    #[tokio::test]
    async fn scanned_records_and_blocking() {
        let db = Storage::connect(&test_pg()).await.unwrap();
        clean(&db).await;
        let hash = [6u8; 20];
        let now = 1_700_000_000i64;

        assert!(!db.scan_blocked(&hash, now).await.unwrap());

        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + backoff_secs(1),
                failure_reason: Some("timeout".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .await
        .unwrap();
        assert!(db.scan_blocked(&hash, now).await.unwrap());
        assert!(!db.scan_blocked(&hash, now + backoff_secs(1) + 1).await.unwrap());

        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Ok,
            info_bytes: Some(vec![1, 2, 3]),
            raw_name: Some("x".into()),
            last_attempt: now,
        })
        .await
        .unwrap();
        assert_eq!(
            db.scan_status(&hash).await.unwrap(),
            Some(ScannedStatus::Ok)
        );
        assert!(db.scan_blocked(&hash, now).await.unwrap());
        assert!(db.scan_blocked(&hash, now + 100_000).await.unwrap());

        let h2 = [7u8; 20];
        db.record_scanned(&ScannedRecord {
            info_hash: h2,
            status: ScannedStatus::Skipped,
            info_bytes: Some(vec![9]),
            raw_name: None,
            last_attempt: now,
        })
        .await
        .unwrap();
        assert_eq!(
            db.scan_status(&h2).await.unwrap(),
            Some(ScannedStatus::Skipped)
        );
        assert!(db.scan_blocked(&h2, now).await.unwrap());
    }

    #[tokio::test]
    async fn scan_blocked_batch_flags_only_blocked() {
        let db = Storage::connect(&test_pg()).await.unwrap();
        clean(&db).await;
        let now = 1_700_000_000i64;
        let accepted = [1u8; 20];
        let in_backoff = [2u8; 20];
        let fresh = [3u8; 20];

        db.record_scanned(&ScannedRecord {
            info_hash: accepted,
            status: ScannedStatus::Ok,
            info_bytes: Some(vec![1]),
            raw_name: None,
            last_attempt: now,
        })
        .await
        .unwrap();
        db.record_scanned(&ScannedRecord {
            info_hash: in_backoff,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + 60,
                failure_reason: Some("timeout".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .await
        .unwrap();

        let blocked = db
            .scan_blocked_batch(&[accepted, in_backoff, fresh], now)
            .await
            .unwrap();
        assert_eq!(blocked.len(), 2, "accepted + backoff blocked, fresh not");
        assert!(blocked.contains(&accepted));
        assert!(blocked.contains(&in_backoff));
        assert!(!blocked.contains(&fresh));

        assert!(db.scan_blocked_batch(&[], now).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scanned_failure_increments_attempts() {
        let db = Storage::connect(&test_pg()).await.unwrap();
        clean(&db).await;
        let hash = [8u8; 20];
        let now = 1_700_000_000i64;
        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + backoff_secs(1),
                failure_reason: Some("connect_refused".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .await
        .unwrap();
        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 2,
                next_attempt: now + backoff_secs(2),
                failure_reason: Some("no_ut_metadata".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now + 600,
        })
        .await
        .unwrap();
        assert_eq!(
            db.scan_status(&hash).await.unwrap(),
            Some(ScannedStatus::Failed {
                attempts: 2,
                next_attempt: now + backoff_secs(2),
                failure_reason: Some("no_ut_metadata".into()),
            })
        );
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(3), 240);
        assert!(backoff_secs(100) <= 6 * 3600);
        assert_eq!(backoff_secs(100), backoff_secs(200));
    }
}
