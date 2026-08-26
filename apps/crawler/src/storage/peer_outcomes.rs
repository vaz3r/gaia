use crate::krpc::Infohash;
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct PeerOutcome {
    pub ih: Infohash,
    pub peer: String,
    pub source: String,
    pub transport: String,
    pub result: String,
    pub client: Option<String>,
    pub phase: Option<String>,
    pub elapsed_ms: Option<i32>,
}

pub struct PeerOutcomeWriter {
    pool: PgPool,
    buf: Mutex<Vec<PeerOutcome>>,
    chunk_size: usize,
    written: AtomicU64,
    flushing: AtomicBool,
}

impl PeerOutcomeWriter {
    pub fn new(pool: PgPool, chunk_size: usize) -> Self {
        PeerOutcomeWriter {
            pool,
            buf: Mutex::new(Vec::with_capacity(8192)),
            chunk_size: chunk_size.max(1),
            written: AtomicU64::new(0),
            flushing: AtomicBool::new(false),
        }
    }

    pub fn push(&self, outcome: PeerOutcome) {
        self.buf
            .lock()
            .expect("peer outcome writer poisoned")
            .push(outcome);
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub async fn flush(&self) {
        if self.flushing.swap(true, Ordering::Relaxed) {
            return;
        }
        let batch = {
            let mut buf = self.buf.lock().expect("peer outcome writer poisoned");
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
                    tracing::warn!(error = %e, "peer_outcomes: begin tx failed");
                    continue;
                }
            };
            let ihs: Vec<&[u8]> = chunk.iter().map(|o| o.ih.as_slice()).collect();
            let peers: Vec<&str> = chunk.iter().map(|o| o.peer.as_str()).collect();
            let sources: Vec<&str> = chunk.iter().map(|o| o.source.as_str()).collect();
            let transports: Vec<&str> = chunk.iter().map(|o| o.transport.as_str()).collect();
            let results: Vec<&str> = chunk.iter().map(|o| o.result.as_str()).collect();
            let clients: Vec<Option<&str>> = chunk.iter().map(|o| o.client.as_deref()).collect();
            let phases: Vec<Option<&str>> = chunk.iter().map(|o| o.phase.as_deref()).collect();
            let elapsed: Vec<Option<i32>> = chunk.iter().map(|o| o.elapsed_ms).collect();
            let _ = sqlx::query(
                "INSERT INTO fetch_peer_outcomes (infohash, peer, source, transport, result, client, phase, elapsed_ms) \
                 SELECT u.ih, u.peer, u.source, u.transport, u.result, u.client, u.phase, u.elapsed_ms \
                 FROM UNNEST($1::bytea[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[], $8::int4[]) \
                 AS u(ih, peer, source, transport, result, client, phase, elapsed_ms)",
            )
            .bind(&ihs)
            .bind(&peers)
            .bind(&sources)
            .bind(&transports)
            .bind(&results)
            .bind(&clients)
            .bind(&phases)
            .bind(&elapsed)
            .execute(&mut *tx)
            .await;
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, "peer_outcomes: commit failed");
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
