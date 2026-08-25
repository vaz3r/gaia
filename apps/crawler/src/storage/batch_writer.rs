use crate::krpc::Infohash;
use crate::storage::torrents::parse_info_dict;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

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
    max_retries: i32,
    flush_chunk: usize,
    flushing: AtomicBool,
    jobs_written: AtomicU64,
    torrents_written: AtomicU64,
}

impl BatchWriter {
    pub fn new(pool: PgPool, backoffs: Vec<Duration>, max_retries: i32, flush_chunk: usize) -> Self {
        BatchWriter {
            pool,
            jobs: Mutex::new(Vec::with_capacity(4096)),
            torrents: Mutex::new(Vec::with_capacity(4096)),
            backoffs,
            max_retries,
            flush_chunk: flush_chunk.max(1),
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

        let mut verified_ihs: Vec<Vec<u8>> = Vec::new();
        if !job_batch.is_empty() {
            let failed_updates: Vec<&JobUpdate> = job_batch
                .iter()
                .filter(|u| matches!(u, JobUpdate::Failed(_, _)))
                .collect();
            if !failed_updates.is_empty()
                && flush_jobs(
                    &self.pool,
                    &failed_updates,
                    self.flush_chunk,
                    self.max_retries,
                    &self.backoffs,
                )
                .await
            {
                self.jobs_written
                    .fetch_add(failed_updates.len() as u64, Ordering::Relaxed);
            }
            verified_ihs = job_batch
                .iter()
                .filter_map(|u| match u {
                    JobUpdate::Verified(ih) => Some(ih.as_slice().to_vec()),
                    _ => None,
                })
                .collect();
        }
        if !torrent_batch.is_empty() {
            if flush_torrents(&self.pool, &torrent_batch, self.flush_chunk).await {
                self.torrents_written
                    .fetch_add(torrent_batch.len() as u64, Ordering::Relaxed);
            }
        }
        if !verified_ihs.is_empty() {
            delete_verified(&self.pool, &verified_ihs, self.flush_chunk).await;
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

async fn flush_jobs(
    pool: &PgPool,
    batch: &[&JobUpdate],
    flush_chunk: usize,
    max_retries: i32,
    backoffs: &[Duration],
) -> bool {
    let failed_ihs: Vec<&[u8]> = batch
        .iter()
        .filter_map(|u| match u {
            JobUpdate::Failed(ih, _) => Some(ih.as_slice()),
            _ => None,
        })
        .collect();

    let retry_map: HashMap<Vec<u8>, i32> = if !failed_ihs.is_empty() {
        let mut map = HashMap::new();
        for chunk in failed_ihs.chunks(flush_chunk) {
            let chunk_vec: Vec<Vec<u8>> = chunk.iter().map(|b| b.to_vec()).collect();
            let rows = sqlx::query(
                "SELECT infohash, retry_count FROM verification_jobs WHERE infohash = ANY($1)",
            )
            .bind(&chunk_vec)
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
                    return false;
                }
            }
        }
        map
    } else {
        HashMap::new()
    };

    let base_backoff = backoffs.first().copied().unwrap_or(Duration::from_secs(60));
    let max_backoff = backoffs.last().copied().unwrap_or(Duration::from_secs(43200));

    let mut seen_ihs: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    for chunk in batch.chunks(flush_chunk) {
        let mut resolved: Vec<ResolvedJob<'_>> = Vec::with_capacity(chunk.len());

        for update in chunk {
            match update {
                JobUpdate::Failed(ih, error) => {
                    let raw = ih.as_slice().to_vec();
                    if !seen_ihs.insert(raw.clone()) {
                        continue;
                    }
                    let current_rc = retry_map.get(&raw).copied().unwrap_or(0);
                    let new_count = current_rc + 1;
                    let terminal = new_count >= max_retries
                        || (error == "no_peers" && current_rc >= 1);
                    if terminal {
                        resolved.push(ResolvedJob {
                            ih: ih.as_slice(),
                            status: "dead",
                            retry_count: Some(new_count),
                            next_retry_at: None,
                            last_error: Some(error),
                        });
                    } else {
                        let idx = current_rc.max(0) as u32;
                        let delay = base_backoff
                            .checked_mul(1u32.checked_shl(idx).unwrap_or(u32::MAX))
                            .unwrap_or(max_backoff);
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
                JobUpdate::Verified(_) => {}
            }
        }

        if resolved.is_empty() {
            continue;
        }

        let now = Utc::now();
        let n = resolved.len();
        let param_count = n * 6;
        let mut sql = String::with_capacity(256 + param_count * 4);
        sql.push_str("INSERT INTO verification_jobs (infohash, status, retry_count, next_retry_at, last_error, updated_at) VALUES ");
        for i in 0..n {
            let base = i * 6;
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, ${})",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
                base + 6,
            ));
        }
        sql.push_str(
            " ON CONFLICT (infohash) DO UPDATE SET \
             status = EXCLUDED.status, \
             retry_count = COALESCE(EXCLUDED.retry_count, verification_jobs.retry_count), \
             next_retry_at = EXCLUDED.next_retry_at, \
             last_error = COALESCE(EXCLUDED.last_error, verification_jobs.last_error), \
             updated_at = now()",
        );

        let mut q = sqlx::query(&sql);
        for r in &resolved {
            q = q
                .bind(r.ih)
                .bind(r.status)
                .bind(r.retry_count)
                .bind(r.next_retry_at)
                .bind(r.last_error)
                .bind(now);
        }

        match q.execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    chunk_size = chunk.len(),
                    "batch: upsert verification_jobs failed"
                );
                return false;
            }
        }
    }
    true
}

async fn flush_torrents(pool: &PgPool, batch: &[TorrentEntry], flush_chunk: usize) -> bool {
    let mut seen: std::collections::HashSet<[u8; 20]> = std::collections::HashSet::new();
    let unique: Vec<&TorrentEntry> = batch.iter().filter(|e| seen.insert(e.ih)).collect();
    for chunk in unique.chunks(flush_chunk) {
        let now = Utc::now();
        let n = chunk.len();
        let param_count = n * 7;
        let mut sql = String::with_capacity(256 + param_count * 4);
        sql.push_str("INSERT INTO torrents (infohash, name, piece_length, total_size, file_count, files, verified_at) VALUES ");
        for i in 0..n {
            let base = i * 7;
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, ${}, ${})",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
                base + 6,
                base + 7,
            ));
        }
        sql.push_str(
            " ON CONFLICT (infohash) DO UPDATE SET \
             name = EXCLUDED.name, piece_length = EXCLUDED.piece_length, \
             total_size = EXCLUDED.total_size, file_count = EXCLUDED.file_count, \
             files = EXCLUDED.files, verified_at = now()",
        );

        let mut q = sqlx::query(&sql);
        for &e in chunk {
            q = q
                .bind(e.ih.as_slice())
                .bind(e.name.as_deref())
                .bind(e.piece_length)
                .bind(e.total_size)
                .bind(e.file_count)
                .bind(e.files.as_ref())
                .bind(now);
        }

        match q.execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                for t in chunk {
                    let files_json = t
                        .files
                        .as_ref()
                        .and_then(|f| serde_json::to_string(f).ok())
                        .unwrap_or_default();
                    tracing::warn!(
                        error = %e,
                        ih = %crate::trace::hex_encode(&t.ih),
                        name = t.name.as_deref().unwrap_or(""),
                        files = %files_json,
                        "batch: upsert torrents failed"
                    );
                }
                return false;
            }
        }
    }
    true
}

async fn delete_verified(pool: &PgPool, infohashes: &[Vec<u8>], flush_chunk: usize) {
    for chunk in infohashes.chunks(flush_chunk) {
        match sqlx::query("DELETE FROM verification_jobs WHERE infohash = ANY($1)")
            .bind(chunk)
            .execute(pool)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "batch: delete verified jobs failed");
            }
        }
    }
}
