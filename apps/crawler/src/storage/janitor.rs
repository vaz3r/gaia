use sqlx::PgPool;

pub struct JanitorConfig {
    pub dead_retention_secs: u64,
    pub verified_retention_secs: u64,
    pub batch_size: i64,
    pub batch_sleep_ms: u64,
}

pub async fn run(pool: &PgPool, cfg: &JanitorConfig) {
    cleanup_dead(pool, cfg).await;
    cleanup_verified(pool, cfg).await;
}

async fn cleanup_dead(pool: &PgPool, cfg: &JanitorConfig) {
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
        let result = sqlx::query(&sql)
            .bind(cfg.batch_size)
            .execute(pool)
            .await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
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
    if total > 0 {
        tracing::info!(deleted = total, "janitor: dead rows cleaned");
    }
}

async fn cleanup_verified(pool: &PgPool, cfg: &JanitorConfig) {
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
        let result = sqlx::query(&sql)
            .bind(cfg.batch_size)
            .execute(pool)
            .await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
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
    if total > 0 {
        tracing::info!(deleted = total, "janitor: verified rows cleaned");
    }
}
