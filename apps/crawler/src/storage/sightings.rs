use crate::harvest::Source;
use crate::krpc::Infohash;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct SightingWriter {
    pool: PgPool,
    buf: Mutex<Vec<(Infohash, Source)>>,
    written: AtomicU64,
}

impl SightingWriter {
    pub fn new(pool: PgPool) -> Self {
        SightingWriter {
            pool,
            buf: Mutex::new(Vec::with_capacity(4096)),
            written: AtomicU64::new(0),
        }
    }

    pub fn push(&self, ih: Infohash, source: Source) {
        self.buf
            .lock()
            .expect("sighting writer poisoned")
            .push((ih, source));
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub async fn flush(&self) {
        let batch = {
            let mut buf = self.buf.lock().expect("sighting writer poisoned");
            if buf.is_empty() {
                return;
            }
            std::mem::take(&mut *buf)
        };
        for chunk in batch.chunks(256) {
            let mut tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::warn!(error = %e, "sightings: begin tx failed");
                    continue;
                }
            };
            let ihs: Vec<&[u8]> = chunk.iter().map(|(ih, _)| ih.as_slice()).collect();
            let sources: Vec<String> = chunk
                .iter()
                .map(|(_, s)| format!("{{\"{}\":1}}", s.tag()))
                .collect();
            let _ = sqlx::query(
                "INSERT INTO infohash_sightings (infohash, source_counts) \
                 SELECT u.ih, u.sc::jsonb FROM UNNEST($1::bytea[], $2::text[]) AS u(ih, sc) \
                 ON CONFLICT (infohash) DO UPDATE SET \
                 last_seen = now(), total_seen = infohash_sightings.total_seen + 1, \
                 source_counts = infohash_sightings.source_counts || EXCLUDED.source_counts",
            )
            .bind(&ihs)
            .bind(&sources)
            .execute(&mut *tx)
            .await;
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, "sightings: commit failed");
            }
        }
        self.written
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
    }

    pub async fn run(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            self.flush().await;
        }
    }
}
