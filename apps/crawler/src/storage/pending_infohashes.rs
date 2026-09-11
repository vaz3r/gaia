use crate::krpc::Infohash;
use crate::metrics::{Add1, Metrics};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const MAX_QUEUE_LEN: usize = 25_000;

pub struct PendingInfohashWriter {
    pool: PgPool,
    buf: Mutex<Vec<([u8; 20], &'static str)>>,
    chunk_size: usize,
    written: AtomicU64,
    flushing: AtomicBool,
}

impl PendingInfohashWriter {
    pub fn new(pool: PgPool, chunk_size: usize) -> Self {
        PendingInfohashWriter {
            pool,
            buf: Mutex::new(Vec::with_capacity(4096)),
            chunk_size: chunk_size.max(1),
            written: AtomicU64::new(0),
            flushing: AtomicBool::new(false),
        }
    }

    pub fn push(&self, ih: Infohash, source: &'static str) {
        let mut buf = self.buf.lock().expect("pending infohash writer poisoned");
        if buf.len() < MAX_QUEUE_LEN {
            buf.push((ih, source));
        }
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub async fn flush(&self) {
        if self.flushing.swap(true, Ordering::Relaxed) {
            return;
        }
        let batch = {
            let mut buf = self.buf.lock().expect("pending infohash writer poisoned");
            if buf.is_empty() {
                self.flushing.store(false, Ordering::Relaxed);
                return;
            }
            std::mem::take(&mut *buf)
        };
        for chunk in batch.chunks(self.chunk_size) {
            let mut tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::warn!(error = %e, "pending_infohashes: begin tx failed");
                    continue;
                }
            };
            let ihs: Vec<&[u8]> = chunk.iter().map(|(ih, _)| ih.as_slice()).collect();
            let sources: Vec<&str> = chunk.iter().map(|(_, s)| *s).collect();
            let _ = sqlx::query(
                "INSERT INTO pending_infohashes (infohash, source) \
                 SELECT u.ih, u.source FROM UNNEST($1::bytea[], $2::text[]) AS u(ih, source) \
                 ON CONFLICT (infohash) DO NOTHING",
            )
            .bind(&ihs)
            .bind(&sources)
            .execute(&mut *tx)
            .await;
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, "pending_infohashes: commit failed");
            }
        }
        self.written
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
        self.flushing.store(false, Ordering::Relaxed);
    }

    pub async fn run(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            self.flush().await;
        }
    }
}

pub struct PendingInfohashScheduler {
    pool: PgPool,
    claim_limit: i64,
    interval: Duration,
}

impl PendingInfohashScheduler {
    pub fn new(pool: PgPool, claim_limit: usize, interval: Duration) -> Self {
        PendingInfohashScheduler {
            pool,
            claim_limit: claim_limit.max(1) as i64,
            interval,
        }
    }

    pub async fn run(self, tx: mpsc::Sender<Infohash>, metrics: Arc<Metrics>) {
        let mut tick = tokio::time::interval(self.interval);
        loop {
            tick.tick().await;
            if (tx.capacity() as i64) < self.claim_limit {
                metrics.pending_buffer_skipped_backpressure.add(1);
                continue;
            }
            match self.claim_due().await {
                Ok(ihs) => {
                    let count = ihs.len() as u64;
                    for ih in ihs {
                        if tx.send(ih).await.is_err() {
                            break;
                        }
                    }
                    metrics.pending_buffer_replayed.add(count);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "pending_infohashes: claim failed");
                }
            }
        }
    }

    async fn claim_due(&self) -> Result<Vec<Infohash>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let rows: Vec<(Vec<u8>,)> = sqlx::query_as(
            "DELETE FROM pending_infohashes \
             WHERE ctid = ANY( \
                 SELECT ctid FROM pending_infohashes \
                 ORDER BY created_at ASC \
                 LIMIT $1 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING infohash",
        )
        .bind(self.claim_limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut ihs = Vec::with_capacity(rows.len());
        for (ih_bytes,) in rows {
            if ih_bytes.len() == 20 {
                let mut ih = [0u8; 20];
                ih.copy_from_slice(&ih_bytes);
                ihs.push(ih);
            }
        }
        Ok(ihs)
    }
}
