use crate::krpc::Infohash;
use sqlx::PgPool;
use sqlx::Row;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

pub const MAX_RETRIES: i32 = 4;

pub struct VerifyStore {
    pool: PgPool,
    backoffs: Vec<Duration>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl VerifyStore {
    pub fn new(pool: PgPool, backoffs: Vec<Duration>) -> Self {
        VerifyStore { pool, backoffs }
    }

    pub async fn mark_verified(&self, ih: Infohash) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO verification_jobs (infohash, status, updated_at) VALUES ($1, 'verified', now()) \
             ON CONFLICT (infohash) DO UPDATE SET status = 'verified', updated_at = now()",
        )
        .bind(ih.as_slice())
        .execute(&self.pool)
        .await?;
        crate::trace_lifecycle!(&ih, "job_update", stream = "db", status = "verified");
        Ok(())
    }

    pub async fn mark_failed(&self, ih: Infohash, error: &str) -> Result<(), sqlx::Error> {
        crate::trace_lifecycle!(&ih, "job_update", stream = "db", status = "failed", result = error);
        let row = sqlx::query("SELECT retry_count FROM verification_jobs WHERE infohash = $1")
            .bind(ih.as_slice())
            .fetch_optional(&self.pool)
            .await?;
        let retry_count: i32 = match row {
            Some(r) => r.get(0),
            None => 0,
        };
        let new_count = retry_count + 1;
        if new_count >= MAX_RETRIES {
            sqlx::query(
                "INSERT INTO verification_jobs (infohash, status, retry_count, next_retry_at, last_error, updated_at) \
                 VALUES ($1, 'dead', $2, NULL, $4, now()) \
                 ON CONFLICT (infohash) DO UPDATE SET \
                 status = 'dead', retry_count = $2, next_retry_at = NULL, \
                 last_error = $4, updated_at = now()",
            )
            .bind(ih.as_slice())
            .bind(new_count)
            .bind(error)
            .execute(&self.pool)
            .await?;
        } else {
            let idx = retry_count.max(0) as usize;
            let delay = self
                .backoffs
                .get(idx)
                .copied()
                .unwrap_or(Duration::from_secs(365 * 86400));
            let next = now_secs() + delay.as_secs();
            sqlx::query(
                "INSERT INTO verification_jobs (infohash, status, retry_count, next_retry_at, last_error, updated_at) \
                 VALUES ($1, 'failed', $2, to_timestamp($3), $4, now()) \
                 ON CONFLICT (infohash) DO UPDATE SET \
                 status = 'failed', retry_count = $2, next_retry_at = to_timestamp($3), \
                 last_error = $4, updated_at = now()",
            )
            .bind(ih.as_slice())
            .bind(new_count)
            .bind(next as f64)
            .bind(error)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn claim_due(&self, limit: i64) -> Result<Vec<Infohash>, sqlx::Error> {
        let fresh_limit = (limit * 7 / 10).max(1);
        let retry_limit = (limit - fresh_limit).max(1);
        let rows = sqlx::query(
            "WITH fresh AS (
                SELECT infohash FROM verification_jobs
                WHERE status = 'pending'
                ORDER BY next_retry_at NULLS FIRST
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            ),
            retries AS (
                SELECT infohash FROM verification_jobs
                WHERE status = 'failed'
                  AND (next_retry_at IS NULL OR next_retry_at <= now())
                  AND retry_count < $3
                ORDER BY retry_count ASC, next_retry_at ASC
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            UPDATE verification_jobs SET status = 'verifying', updated_at = now()
            WHERE infohash IN (SELECT infohash FROM fresh UNION SELECT infohash FROM retries)
            RETURNING infohash, retry_count",
        )
        .bind(fresh_limit)
        .bind(retry_limit)
        .bind(MAX_RETRIES)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: Vec<u8> = row.get(0);
            let retry_count: i32 = row.get(1);
            if let Ok(ih) = <[u8; 20]>::try_from(raw.as_slice()) {
                crate::trace_lifecycle!(&ih, "claimed", stream = "db", attempt = retry_count as u32);
                out.push(ih);
            }
        }
        Ok(out)
    }

    async fn reset_stale_verifying(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE verification_jobs SET status = 'pending', next_retry_at = now(), updated_at = now() \
             WHERE status = 'verifying' AND updated_at < now() - interval '5 minutes'",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn run_scheduler(self: Arc<Self>, tx: mpsc::Sender<Infohash>, interval: Duration) {
        match self.reset_stale_verifying().await {
            Ok(n) => tracing::info!(
                recovered_jobs = n,
                "verification scheduler: recovered stale verifying jobs"
            ),
            Err(e) => tracing::warn!(error = %e, "verification scheduler: recovery failed"),
        }
        let mut tick = tokio::time::interval(interval);
        let mut stale_tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    match self.claim_due(1000).await {
                        Ok(due) => {
                            if !due.is_empty() {
                                tracing::info!(
                                    due = due.len(),
                                    "verification scheduler: re-injecting jobs"
                                );
                                for ih in due {
                                    let _ = tx.send(ih).await;
                                }
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "verification scheduler: claim failed"),
                    }
                }
                _ = stale_tick.tick() => {
                    match self.reset_stale_verifying().await {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(recovered = n, "scheduler: reset stale verifying jobs"),
                        Err(e) => tracing::warn!(error = %e, "scheduler: stale recovery failed"),
                    }
                }
            }
        }
    }
}
