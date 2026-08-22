use sqlx::PgPool;

const BATCH_SIZE: i64 = 50_000;

pub async fn run(pool: &PgPool) {
    cleanup_dead(pool).await;
    cleanup_verified(pool).await;
}

async fn cleanup_dead(pool: &PgPool) {
    let mut total: i64 = 0;
    loop {
        let result = sqlx::query(
            "DELETE FROM verification_jobs \
             WHERE ctid = ANY( \
                 SELECT ctid FROM verification_jobs \
                 WHERE status = 'dead' AND updated_at < now() - interval '1 day' \
                 LIMIT $1 \
             )",
        )
        .bind(BATCH_SIZE)
        .execute(pool)
        .await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                if n == 0 {
                    break;
                }
                if n < BATCH_SIZE {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

async fn cleanup_verified(pool: &PgPool) {
    let mut total: i64 = 0;
    loop {
        let result = sqlx::query(
            "DELETE FROM verification_jobs \
             WHERE ctid = ANY( \
                 SELECT ctid FROM verification_jobs \
                 WHERE status = 'verified' AND updated_at < now() - interval '1 hour' \
                 LIMIT $1 \
             )",
        )
        .bind(BATCH_SIZE)
        .execute(pool)
        .await;

        match result {
            Ok(r) => {
                let n = r.rows_affected() as i64;
                total += n;
                if n == 0 {
                    break;
                }
                if n < BATCH_SIZE {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
