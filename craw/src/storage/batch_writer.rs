use crate::krpc::Infohash;
use crate::storage::jobs::MAX_RETRIES;
use crate::storage::torrents::parse_info_dict;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::Row;
use sqlx::query_builder::QueryBuilder;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const FLUSH_CHUNK: usize = 5000;
const BUFFER_MAX: usize = 8192;

pub enum JobUpdate {
    Verified(Infohash),
    Failed(Infohash, String),
}

pub struct TorrentEntry {
    pub ih: Infohash,
    pub name: Option<String>,
    pub piece_length: Option<i64>,
    pub total_size: Option<i64>,
    pub file_count: Option<i64>,
    pub files: Option<serde_json::Value>,
}

struct ResolvedJob<'a> {
    ih: &'a [u8],
    status: &'static str,
    retry_count: Option<i32>,
    next_retry_at: Option<DateTime<Utc>>,
    last_error: Option<&'a str>,
}

pub struct BatchWriter {
    pool: PgPool,
    jobs: Mutex<Vec<JobUpdate>>,
    torrents: Mutex<Vec<TorrentEntry>>,
    backoffs: Vec<Duration>,
    flushing: AtomicBool,
    jobs_written: AtomicU64,
    torrents_written: AtomicU64,
}

impl BatchWriter {
    pub fn new(pool: PgPool, backoffs: Vec<Duration>) -> Self {
        BatchWriter {
            pool,
            jobs: Mutex::new(Vec::with_capacity(4096)),
            torrents: Mutex::new(Vec::with_capacity(4096)),
            backoffs,
            flushing: AtomicBool::new(false),
            jobs_written: AtomicU64::new(0),
            torrents_written: AtomicU64::new(0),
        }
    }

    pub fn push_verified(&self, ih: Infohash) {
        let mut buf = self.jobs.lock().expect("batch writer jobs poisoned");
        buf.push(JobUpdate::Verified(ih));
    }

    pub fn push_failed(&self, ih: Infohash, error: &str) {
        let mut buf = self.jobs.lock().expect("batch writer jobs poisoned");
        buf.push(JobUpdate::Failed(ih, error.to_owned()));
    }

    pub fn push_torrent(&self, ih: Infohash, metadata: &[u8]) {
        let p = parse_info_dict(metadata);
        let mut buf = self.torrents.lock().expect("batch writer torrents poisoned");
        buf.push(TorrentEntry {
            ih,
            name: p.name,
            piece_length: p.piece_length,
            total_size: p.total_size,
            file_count: p.file_count,
            files: p.files,
        });
    }

    pub fn jobs_written(&self) -> u64 {
        self.jobs_written.load(Ordering::Relaxed)
    }

    pub fn torrents_written(&self) -> u64 {
        self.torrents_written.load(Ordering::Relaxed)
    }

    pub async fn flush(&self) {
        if self
            .flushing
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let job_batch: Vec<JobUpdate> = {
            let mut buf = self.jobs.lock().expect("batch writer jobs poisoned");
            buf.drain(..).collect()
        };
        let torrent_batch: Vec<TorrentEntry> = {
            let mut buf = self.torrents.lock().expect("batch writer torrents poisoned");
            buf.drain(..).collect()
        };

        if !job_batch.is_empty() {
            if flush_jobs(&self.pool, &job_batch).await {
                self.jobs_written
                    .fetch_add(job_batch.len() as u64, Ordering::Relaxed);
            }
        }
        if !torrent_batch.is_empty() {
            if flush_torrents(&self.pool, &torrent_batch).await {
                self.torrents_written
                    .fetch_add(torrent_batch.len() as u64, Ordering::Relaxed);
            }
        }

        self.flushing.store(false, Ordering::Release);
    }

    pub async fn run(self: Arc<Self>, interval: Duration, mut shutdown: broadcast::Receiver<()>) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = tick.tick() => self.flush().await,
                _ = shutdown.recv() => {
                    self.flush().await;
                    break;
                }
            }
        }
    }
}

async fn flush_jobs(pool: &PgPool, batch: &[JobUpdate]) -> bool {
    let failed_ihs: Vec<&[u8]> = batch
        .iter()
        .filter_map(|u| match u {
            JobUpdate::Failed(ih, _) => Some(ih.as_slice()),
            _ => None,
        })
        .collect();

    let retry_map: HashMap<Vec<u8>, i32> = if !failed_ihs.is_empty() {
        let mut map = HashMap::new();
        for chunk in failed_ihs.chunks(FLUSH_CHUNK) {
            let rows = sqlx::query(
                "SELECT infohash, retry_count FROM verification_jobs WHERE infohash = ANY($1)",
            )
            .bind(chunk)
            .fetch_all(pool)
            .await;
            match rows {
                Ok(rows) => {
                    for row in rows {
                        let ih: Vec<u8> = row.get(0);
                        let rc: i32 = row.get(1);
                        map.insert(ih, rc);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "batch: select retry_counts failed");
                }
            }
        }
        map
    } else {
        HashMap::new()
    };

    for chunk in batch.chunks(FLUSH_CHUNK) {
        let mut resolved: Vec<ResolvedJob<'_>> = Vec::with_capacity(chunk.len());

        for update in chunk {
            match update {
                JobUpdate::Verified(ih) => {
                    resolved.push(ResolvedJob {
                        ih: ih.as_slice(),
                        status: "verified",
                        retry_count: Some(0),
                        next_retry_at: None,
                        last_error: None,
                    });
                }
                JobUpdate::Failed(ih, error) => {
                    let raw = ih.as_slice().to_vec();
                    let current_rc = retry_map.get(&raw).copied().unwrap_or(0);
                    let new_count = current_rc + 1;
                    if new_count >= MAX_RETRIES {
                        resolved.push(ResolvedJob {
                            ih: ih.as_slice(),
                            status: "dead",
                            retry_count: Some(new_count),
                            next_retry_at: None,
                            last_error: Some(error),
                        });
                    } else {
                        let idx = current_rc.max(0) as usize;
                        let delay = Duration::from_secs(60)
                            .checked_mul(1u32.checked_shl(idx as u32).unwrap_or(u32::MAX))
                            .unwrap_or(Duration::from_secs(43200));
                        let next = Utc::now() + chrono::Duration::from_std(delay).unwrap();
                        resolved.push(ResolvedJob {
                            ih: ih.as_slice(),
                            status: "failed",
                            retry_count: Some(new_count),
                            next_retry_at: Some(next),
                            last_error: Some(error),
                        });
                    }
                }
            }
        }

        if resolved.is_empty() {
            continue;
        }

        let mut qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO verification_jobs (infohash, status, retry_count, next_retry_at, last_error, updated_at) ",
        );
        qb.push_values(resolved.iter(), |mut b, r| {
            b.push_bind(r.ih);
            b.push_bind(r.status);
            b.push_bind(r.retry_count);
            b.push_bind(r.next_retry_at);
            b.push_bind(r.last_error);
            b.push("now()");
        });
        qb.push(
            " ON CONFLICT (infohash) DO UPDATE SET \
             status = EXCLUDED.status, \
             retry_count = COALESCE(EXCLUDED.retry_count, verification_jobs.retry_count), \
             next_retry_at = EXCLUDED.next_retry_at, \
             last_error = COALESCE(EXCLUDED.last_error, verification_jobs.last_error), \
             updated_at = now()",
        );

        match qb.build().execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "batch: upsert verification_jobs failed");
                return false;
            }
        }
    }
    true
}

async fn flush_torrents(pool: &PgPool, batch: &[TorrentEntry]) -> bool {
    for chunk in batch.chunks(FLUSH_CHUNK) {
        let mut qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO torrents (infohash, name, piece_length, total_size, file_count, files, verified_at) ",
        );
        qb.push_values(chunk.iter(), |mut b, e| {
            b.push_bind(e.ih.as_slice());
            b.push_bind(e.name.as_deref());
            b.push_bind(e.piece_length);
            b.push_bind(e.total_size);
            b.push_bind(e.file_count);
            b.push_bind(e.files.as_ref());
            b.push("now()");
        });
        qb.push(
            " ON CONFLICT (infohash) DO UPDATE SET \
             name = EXCLUDED.name, piece_length = EXCLUDED.piece_length, \
             total_size = EXCLUDED.total_size, file_count = EXCLUDED.file_count, \
             files = EXCLUDED.files, verified_at = now()",
        );

        match qb.build().execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "batch: upsert torrents failed");
                return false;
            }
        }
    }
    true
}
