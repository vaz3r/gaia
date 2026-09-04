use sqlx::PgPool;
use std::time::Instant;

pub struct JanitorConfig {
    pub dead_retention_secs: u64,
    pub verified_retention_secs: u64,
    pub peer_outcomes_retention_secs: u64,
    pub sightings_single_seen_retention_secs: u64,
    pub sightings_max_retention_secs: u64,
    pub batch_size: i64,
    pub batch_sleep_ms: u64,
}

#[derive(Debug, Default)]
pub struct JanitorReport {
    pub dead_deleted: i64,
    pub verified_deleted: i64,
    pub peer_outcomes_deleted: i64,
    pub sightings_deleted: i64,
    pub dead_batches: u32,
    pub verified_batches: u32,
    pub peer_outcomes_batches: u32,
    pub sightings_batches: u32,
    pub elapsed_ms: u64,
}

pub async fn run(pool: &PgPool, cfg: &JanitorConfig) -> JanitorReport {
    let start = Instant::now();
    let mut report = JanitorReport::default();
    report.dead_deleted = cleanup_dead(pool, cfg, &mut report).await;
    report.verified_deleted = cleanup_verified(pool, cfg, &mut report).await;
    report.peer_outcomes_deleted = cleanup_peer_outcomes(pool, cfg, &mut report).await;
    report.sightings_deleted = cleanup_sightings(pool, cfg, &mut report).await;
    report.elapsed_ms = start.elapsed().as_millis() as u64;
    if report.dead_deleted > 0
        || report.verified_deleted > 0
        || report.peer_outcomes_deleted > 0
        || report.sightings_deleted > 0
    {
        tracing::info!(
            dead_deleted = report.dead_deleted,
            dead_batches = report.dead_batches,
            verified_deleted = report.verified_deleted,
            verified_batches = report.verified_batches,
            peer_outcomes_deleted = report.peer_outcomes_deleted,
            peer_outcomes_batches = report.peer_outcomes_batches,
            sightings_deleted = report.sightings_deleted,
            sightings_batches = report.sightings_batches,
            elapsed_ms = report.elapsed_ms,
            "janitor: cleanup complete"
        );
    }
    report
}

async fn cleanup_peer_outcomes(pool: &PgPool, cfg: &JanitorConfig, report: &mut JanitorReport) -> i64 {
    let mut total: i64 = 0;
    loop {
        let sql = format!(
            "DELETE FROM fetch_peer_outcomes \
             WHERE ctid = ANY( \
                 SELECT ctid FROM fetch_peer_outcomes \
                 WHERE created_at < now() - interval '{} seconds' \
                 LIMIT $1 \
             )",
            cfg.peer_outcomes_retention_secs
        );
        let result = sqlx::query(&sql).bind(cfg.batch_size).execute(pool).await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                report.peer_outcomes_batches += 1;
                if n == 0 {
                    break;
                }
                if n < cfg.batch_size {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(cfg.batch_sleep_ms)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "janitor: delete peer_outcomes failed");
                break;
            }
        }
    }
    total
}

async fn cleanup_sightings(pool: &PgPool, cfg: &JanitorConfig, report: &mut JanitorReport) -> i64 {
    let mut total: i64 = 0;
    loop {
        // Purge 1-hit wonders older than single_seen_retention OR any sighting older than max_retention
        let sql = format!(
            "DELETE FROM infohash_sightings \
             WHERE ctid = ANY( \
                 SELECT ctid FROM infohash_sightings \
                 WHERE (last_seen < now() - interval '{} seconds' AND total_seen = 1) \
                    OR (last_seen < now() - interval '{} seconds') \
                 LIMIT $1 \
             )",
            cfg.sightings_single_seen_retention_secs,
            cfg.sightings_max_retention_secs
        );
        let result = sqlx::query(&sql).bind(cfg.batch_size).execute(pool).await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                report.sightings_batches += 1;
                if n == 0 {
                    break;
                }
                if n < cfg.batch_size {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(cfg.batch_sleep_ms)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "janitor: delete sightings failed");
                break;
            }
        }
    }
    total
}

async fn cleanup_dead(pool: &PgPool, cfg: &JanitorConfig, report: &mut JanitorReport) -> i64 {
    let mut total: i64 = 0;
    loop {
        let sql = format!(
            "DELETE FROM verification_jobs \
             WHERE ctid = ANY( \
                 SELECT ctid FROM verification_jobs \
                 WHERE status = 'dead' AND updated_at < now() - interval '{} seconds' \
                 LIMIT $1 \
             )",
            cfg.dead_retention_secs
        );
        let result = sqlx::query(&sql).bind(cfg.batch_size).execute(pool).await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                report.dead_batches += 1;
                if n == 0 {
                    break;
                }
                if n < cfg.batch_size {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(cfg.batch_sleep_ms)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "janitor: delete dead failed");
                break;
            }
        }
    }
    total
}

async fn cleanup_verified(pool: &PgPool, cfg: &JanitorConfig, report: &mut JanitorReport) -> i64 {
    let mut total: i64 = 0;
    loop {
        let sql = format!(
            "DELETE FROM verification_jobs \
             WHERE ctid = ANY( \
                 SELECT ctid FROM verification_jobs \
                 WHERE status = 'verified' AND updated_at < now() - interval '{} seconds' \
                 LIMIT $1 \
             )",
            cfg.verified_retention_secs
        );
        let result = sqlx::query(&sql).bind(cfg.batch_size).execute(pool).await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                report.verified_batches += 1;
                if n == 0 {
                    break;
                }
                if n < cfg.batch_size {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(cfg.batch_sleep_ms)).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "janitor: delete verified failed");
                break;
            }
        }
    }
    total
}
