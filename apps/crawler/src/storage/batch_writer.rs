use crate::krpc::Infohash;
use crate::storage::torrents::parse_info_dict;
use chrono::Utc;
use sqlx::PgPool;
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

pub struct BatchWriter {
    pool: PgPool,
    jobs: Mutex<Vec<JobUpdate>>,
    torrents: Mutex<Vec<TorrentEntry>>,
    stable_peers: Mutex<Vec<std::net::SocketAddr>>,
    peer_torrents: Mutex<Vec<(std::net::SocketAddr, Infohash)>>,
    backoffs: Vec<Duration>,
    max_retries: i32,
    no_peers_terminal_on_first: bool,
    no_peers_max_retries: i32,
    no_metadata_max_retries: i32,
    flush_chunk: usize,
    torrent_flush_chunk: usize,
    flushing: AtomicBool,
    jobs_written: AtomicU64,
    torrents_written: AtomicU64,
}

impl BatchWriter {
    pub fn new(
        pool: PgPool,
        backoffs: Vec<Duration>,
        max_retries: i32,
        no_peers_terminal_on_first: bool,
        no_peers_max_retries: i32,
        no_metadata_max_retries: i32,
        flush_chunk: usize,
        torrent_flush_chunk: usize,
    ) -> Self {
        BatchWriter {
            pool,
            jobs: Mutex::new(Vec::with_capacity(4096)),
            torrents: Mutex::new(Vec::with_capacity(4096)),
            stable_peers: Mutex::new(Vec::with_capacity(4096)),
            peer_torrents: Mutex::new(Vec::with_capacity(4096)),
            backoffs,
            max_retries,
            no_peers_terminal_on_first,
            no_peers_max_retries,
            no_metadata_max_retries,
            flush_chunk: flush_chunk.max(1),
            torrent_flush_chunk: torrent_flush_chunk.max(1),
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

    pub fn push_torrent(&self, ih: Infohash, metadata: &[u8], peer_addr: std::net::SocketAddr) {
        let mut stable = self
            .stable_peers
            .lock()
            .expect("batch writer stable_peers poisoned");
        stable.push(peer_addr);
        let mut pt = self
            .peer_torrents
            .lock()
            .expect("batch writer peer_torrents poisoned");
        pt.push((peer_addr, ih));
        let p = parse_info_dict(metadata);
        let mut buf = self
            .torrents
            .lock()
            .expect("batch writer torrents poisoned");
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
        let stable_peers_batch: Vec<std::net::SocketAddr> = {
            let mut buf = self
                .stable_peers
                .lock()
                .expect("batch writer stable_peers poisoned");
            buf.drain(..).collect()
        };
        let peer_torrents_batch: Vec<(std::net::SocketAddr, Infohash)> = {
            let mut buf = self
                .peer_torrents
                .lock()
                .expect("batch writer peer_torrents poisoned");
            buf.drain(..).collect()
        };
        let torrent_batch: Vec<TorrentEntry> = {
            let mut buf = self
                .torrents
                .lock()
                .expect("batch writer torrents poisoned");
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
                    self.no_peers_terminal_on_first,
                    self.no_peers_max_retries,
                    self.no_metadata_max_retries,
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
        if !torrent_batch.is_empty()
            && flush_torrents(&self.pool, &torrent_batch, self.torrent_flush_chunk).await
        {
            self.torrents_written
                .fetch_add(torrent_batch.len() as u64, Ordering::Relaxed);
        }
        if !stable_peers_batch.is_empty() {
            flush_stable_peers(&self.pool, &stable_peers_batch, self.flush_chunk).await;
        }
        if !peer_torrents_batch.is_empty() {
            flush_peer_torrents(&self.pool, &peer_torrents_batch, self.flush_chunk).await;
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
    no_peers_terminal_on_first: bool,
    no_peers_max_retries: i32,
    no_metadata_max_retries: i32,
    backoffs: &[Duration],
) -> bool {
    let base_backoff = backoffs
        .first()
        .copied()
        .unwrap_or(Duration::from_secs(60))
        .as_secs_f64();
    let max_backoff = backoffs
        .last()
        .copied()
        .unwrap_or(Duration::from_secs(43200))
        .as_secs_f64();

    let mut seen_ihs: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let unique_failed: Vec<&JobUpdate> = batch
        .iter()
        .filter(|u| match u {
            JobUpdate::Failed(ih, _) => seen_ihs.insert(ih.as_slice().to_vec()),
            _ => false,
        })
        .copied()
        .collect();

    let no_peers_terminal = if no_peers_terminal_on_first {
        "true"
    } else {
        "false"
    };
    let no_peers_limit = no_peers_max_retries.max(0);
    let no_metadata_limit = no_metadata_max_retries.max(0);
    for chunk in unique_failed.chunks(flush_chunk) {
        let n = chunk.len();
        let mut sql = String::with_capacity(256 + n * 5 * 4);
        sql.push_str("INSERT INTO verification_jobs (infohash, status, retry_count, next_retry_at, last_error, updated_at) VALUES ");
        for i in 0..n {
            let base = i * 5;
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, now())",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
            ));
        }
        // Single statement: the retry_count increment, status transition, and
        // exponential backoff are all computed in PostgreSQL, eliminating the
        // separate SELECT retry_count round-trip (was a 1.3s slow statement).
        sql.push_str(&format!(
            " ON CONFLICT (infohash) DO UPDATE SET \
             status = CASE \
                 WHEN COALESCE(verification_jobs.retry_count, 0) + 1 >= {max_retries} THEN 'dead' \
                 WHEN EXCLUDED.last_error = 'no_peers' AND ({no_peers_terminal} OR COALESCE(verification_jobs.retry_count, 0) + 1 >= {no_peers_limit}) THEN 'dead' \
                 WHEN EXCLUDED.last_error = 'no_metadata' AND {no_metadata_limit} >= 0 AND COALESCE(verification_jobs.retry_count, 0) >= {no_metadata_limit} THEN 'dead' \
                 ELSE 'failed' \
             END, \
             retry_count = COALESCE(verification_jobs.retry_count, 0) + 1, \
             next_retry_at = CASE \
                 WHEN COALESCE(verification_jobs.retry_count, 0) + 1 >= {max_retries} THEN NULL \
                 WHEN EXCLUDED.last_error = 'no_peers' AND ({no_peers_terminal} OR COALESCE(verification_jobs.retry_count, 0) + 1 >= {no_peers_limit}) THEN NULL \
                 WHEN EXCLUDED.last_error = 'no_metadata' AND {no_metadata_limit} >= 0 AND COALESCE(verification_jobs.retry_count, 0) >= {no_metadata_limit} THEN NULL \
                 ELSE now() + make_interval(secs => LEAST(\
{base_backoff}::double precision * power(2::double precision, COALESCE(verification_jobs.retry_count, 0)::double precision),\
{max_backoff}::double precision)) \
             END, \
             last_error = EXCLUDED.last_error, \
             updated_at = now()",
            max_retries = max_retries,
            no_peers_terminal = no_peers_terminal,
            no_peers_limit = no_peers_limit,
            no_metadata_limit = no_metadata_limit,
            base_backoff = base_backoff,
            max_backoff = max_backoff
        ));

        let mut q = sqlx::query(&sql);
        for update in chunk {
            if let JobUpdate::Failed(ih, error) = update {
                // Initial state used only when the row does not yet exist
                // (retry_count 0 => first failure): same decisions as the
                // ON CONFLICT branch but with current_rc = 0.
                let no_peers_terminal = error == "no_peers" && (no_peers_terminal_on_first || no_peers_max_retries <= 1);
                let no_metadata_terminal = error == "no_metadata" && no_metadata_max_retries >= 0;
                let terminal = max_retries <= 1 || no_peers_terminal || no_metadata_terminal;
                let (init_status, init_next) = if terminal {
                    ("dead", None)
                } else {
                    let next = Utc::now()
                        + chrono::Duration::from_std(Duration::from_secs_f64(base_backoff))
                            .unwrap_or_else(|_| chrono::Duration::seconds(60));
                    ("failed", Some(next))
                };
                q = q
                    .bind(ih.as_slice())
                    .bind(init_status)
                    .bind(1i32) // retry_count for a fresh insert = 1
                    .bind(init_next)
                    .bind(error);
            }
        }

        if let Err(e) = q.execute(pool).await {
            tracing::warn!(
                error = %e,
                chunk_size = chunk.len(),
                "batch: upsert verification_jobs failed"
            );
            return false;
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
        sql.push_str(
            "INSERT INTO torrents (infohash, name, piece_length, total_size, file_count, files, verified_at, \
             health_score, popularity_score, swarm_peers, seed_confirmed, last_health_check) VALUES ",
        );
        for i in 0..n {
            let base = i * 7;
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, ${}, ${}, 50, 50, 0, false, now())",
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

async fn flush_stable_peers(pool: &PgPool, peers: &[std::net::SocketAddr], chunk_size: usize) {
    // Deduplicate within the entire batch first to avoid Postgres rejecting
    // "ON CONFLICT DO UPDATE command cannot affect row a second time" when the
    // same (ip, port) appears more than once in a single UNNEST statement.
    let mut counts: std::collections::HashMap<std::net::SocketAddr, i32> =
        std::collections::HashMap::new();
    for addr in peers {
        *counts.entry(*addr).or_insert(0) += 1;
    }

    let deduped: Vec<(std::net::SocketAddr, i32)> = counts.into_iter().collect();

    for chunk in deduped.chunks(chunk_size) {
        let mut ips = Vec::with_capacity(chunk.len());
        let mut ports = Vec::with_capacity(chunk.len());
        let mut hit_counts = Vec::with_capacity(chunk.len());

        for (addr, count) in chunk {
            ips.push(addr.ip().to_string());
            ports.push(addr.port() as i32);
            hit_counts.push(*count);
        }

        let query = r#"
            INSERT INTO stable_peers (ip, port, metadata_provided_count, first_seen, last_seen)
            SELECT u.ip::inet, u.port, u.cnt, now(), now()
            FROM UNNEST($1::text[], $2::int[], $3::int[]) AS u(ip, port, cnt)
            ON CONFLICT (ip, port) DO UPDATE
            SET metadata_provided_count = stable_peers.metadata_provided_count + EXCLUDED.metadata_provided_count,
                last_seen = now()
        "#;
        if let Err(e) = sqlx::query(query)
            .bind(&ips)
            .bind(&ports)
            .bind(&hit_counts)
            .execute(pool)
            .await
        {
            tracing::error!(error = %e, "failed to batch insert stable_peers");
        }
    }
}

async fn flush_peer_torrents(
    pool: &PgPool,
    records: &[(std::net::SocketAddr, Infohash)],
    chunk_size: usize,
) {
    let mut seen: std::collections::HashSet<(std::net::SocketAddr, Infohash)> =
        std::collections::HashSet::new();
    let deduped: Vec<&(std::net::SocketAddr, Infohash)> =
        records.iter().filter(|r| seen.insert(**r)).collect();

    for chunk in deduped.chunks(chunk_size) {
        let mut ips = Vec::with_capacity(chunk.len());
        let mut ports = Vec::with_capacity(chunk.len());
        let mut ihs = Vec::with_capacity(chunk.len());

        for (addr, ih) in chunk {
            ips.push(addr.ip().to_string());
            ports.push(addr.port() as i32);
            ihs.push(ih.as_slice());
        }

        let query = r#"
            INSERT INTO peer_torrents (peer_ip, peer_port, infohash, verified_at)
            SELECT u.ip::inet, u.port, u.ih, now()
            FROM UNNEST($1::text[], $2::int[], $3::bytea[]) AS u(ip, port, ih)
            ON CONFLICT (peer_ip, peer_port, infohash) DO NOTHING
        "#;
        if let Err(e) = sqlx::query(query)
            .bind(&ips)
            .bind(&ports)
            .bind(&ihs)
            .execute(pool)
            .await
        {
            tracing::error!(error = %e, "failed to batch insert peer_torrents");
        }
    }
}

