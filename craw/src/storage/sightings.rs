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
            for (ih, source) in chunk {
                let tag = source.tag();
                let json = format!("{{\"{tag}\":1}}");
                let _ = sqlx::query(
                    "INSERT INTO infohash_sightings (infohash, source_counts) VALUES ($1, $2::jsonb) \
                     ON CONFLICT (infohash) DO UPDATE SET \
                     last_seen = now(), total_seen = infohash_sightings.total_seen + 1, \
                     source_counts = infohash_sightings.source_counts || $2::jsonb",
                )
                .bind(ih.as_slice())
                .bind(&json)
                .execute(&mut *tx)
                .await;
            }
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
