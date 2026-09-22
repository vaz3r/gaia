use sqlx::PgPool;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct ObservationWriter {
    pool: PgPool,
    buf: Mutex<Vec<Observation>>,
    chunk_size: usize,
    written: AtomicU64,
}

pub struct Observation {
    pub infohash: Vec<u8>,
    pub observation_type: String,
    pub peer_count: i32,
    pub seed_count: i32,
    pub source: String,
    pub latency_ms: Option<i32>,
    pub failure_reason: Option<String>,
}

impl ObservationWriter {
    pub fn new(pool: PgPool, chunk_size: usize) -> Self {
        ObservationWriter {
            pool,
            buf: Mutex::new(Vec::with_capacity(4096)),
            chunk_size: chunk_size.max(1),
            written: AtomicU64::new(0),
        }
    }

    const MAX_QUEUE_LEN: usize = 5_000;

    pub fn push(&self, obs: Observation) {
        let mut buf = self.buf.lock().expect("observation writer poisoned");
        if buf.len() < Self::MAX_QUEUE_LEN {
            buf.push(obs);
        }
    }

    pub fn push_peer_seen(&self, infohash: Vec<u8>, peer_count: i32, seed_count: i32) {
        self.push(Observation {
            infohash,
            observation_type: "peer_seen".to_string(),
            peer_count,
            seed_count,
            source: "crawler".to_string(),
            latency_ms: None,
            failure_reason: None,
        });
    }

    pub fn push_metadata_success(&self, infohash: Vec<u8>, latency_ms: Option<i32>) {
        self.push(Observation {
            infohash,
            observation_type: "metadata_fetch_success".to_string(),
            peer_count: 0,
            seed_count: 0,
            source: "crawler".to_string(),
            latency_ms,
            failure_reason: None,
        });
    }

    pub fn push_metadata_failure(&self, infohash: Vec<u8>, reason: String) {
        self.push(Observation {
            infohash,
            observation_type: "metadata_fetch_failure".to_string(),
            peer_count: 0,
            seed_count: 0,
            source: "crawler".to_string(),
            latency_ms: None,
            failure_reason: Some(reason),
        });
    }

    pub fn push_seed_confirmed(&self, infohash: Vec<u8>) {
        self.push(Observation {
            infohash,
            observation_type: "seed_confirmed".to_string(),
            peer_count: 0,
            seed_count: 1,
            source: "crawler".to_string(),
            latency_ms: None,
            failure_reason: None,
        });
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub async fn flush(&self) {
        let batch = {
            let mut buf = self.buf.lock().expect("observation writer poisoned");
            if buf.is_empty() {
                return;
            }
            std::mem::take(&mut *buf)
        };
        for chunk in batch.chunks(self.chunk_size) {
            let mut tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::warn!(error = %e, "observations: begin tx failed");
                    continue;
                }
            };
            let ihs: Vec<&[u8]> = chunk.iter().map(|o| o.infohash.as_slice()).collect();
            let types: Vec<String> = chunk.iter().map(|o| o.observation_type.clone()).collect();
            let peers: Vec<i32> = chunk.iter().map(|o| o.peer_count).collect();
            let seeds: Vec<i32> = chunk.iter().map(|o| o.seed_count).collect();
            let sources: Vec<String> = chunk.iter().map(|o| o.source.clone()).collect();
            let latencies: Vec<Option<i32>> = chunk.iter().map(|o| o.latency_ms).collect();
            let reasons: Vec<Option<String>> = chunk.iter().map(|o| o.failure_reason.clone()).collect();
            let _ = sqlx::query(
                "INSERT INTO torrent_availability_observations \
                 (infohash, observation_type, peer_count, seed_count, source, latency_ms, failure_reason) \
                 SELECT u.ih, u.obs_type, u.peer_cnt, u.seed_cnt, u.src, u.lat, u.reason \
                 FROM UNNEST($1::bytea[], $2::text[], $3::int4[], $4::int4[], $5::text[], $6::int4[], $7::text[]) \
                 AS u(ih, obs_type, peer_cnt, seed_cnt, src, lat, reason)",
            )
            .bind(&ihs)
            .bind(&types)
            .bind(&peers)
            .bind(&seeds)
            .bind(&sources)
            .bind(&latencies)
            .bind(&reasons)
            .execute(&mut *tx)
            .await;
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, "observations: commit failed");
            }
        }
        self.written
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
    }

    pub async fn run(self: std::sync::Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            self.flush().await;
        }
    }
}
