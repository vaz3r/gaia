pub mod model;
pub mod schema;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub use model::{
    backoff_secs, EMPTY_PEERS_RETRY_SECS, ScannedRecord, ScannedStatus, TorrentRecord,
};

const UPSERT: &str = "
INSERT INTO torrents (info_hash, name, size_bytes, file_count, first_seen, last_seen)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(info_hash) DO UPDATE SET
    name       = excluded.name,
    size_bytes = excluded.size_bytes,
    file_count = excluded.file_count,
    last_seen  = excluded.last_seen
";

const SELECT_COLS: &str = "info_hash, name, size_bytes, file_count, first_seen, last_seen";

/// SQLite-backed storage with WAL mode. A writer connection handles batched
/// upserts; a reader connection serves membership checks and searches so reads
/// never block the write path.
#[derive(Clone)]
pub struct Storage {
    write: Arc<Mutex<Connection>>,
    read: Arc<Mutex<Connection>>,
}

impl Storage {
    /// Open (or create) the database and initialize/migrate the schema.
    pub fn open(path: &str) -> Result<Self> {
        let write = Connection::open(path).with_context(|| format!("open db {path}"))?;
        schema::configure(&write)?;

        let read = Connection::open(path).with_context(|| format!("open db {path}"))?;
        schema::configure(&read)?;

        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read: Arc::new(Mutex::new(read)),
        })
    }

    /// Insert or update a batch of records in a single transaction, preserving
    /// the original `first_seen` on duplicates.
    pub fn insert_batch(&self, records: &[TorrentRecord]) -> Result<()> {
        let mut conn = self.write.lock().unwrap();
        let tx = conn.transaction()?;
        for r in records {
            tx.execute(
                UPSERT,
                params![
                    r.info_hash,
                    r.name,
                    r.size_bytes,
                    r.file_count,
                    r.first_seen,
                    r.last_seen,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The recorded scan status for `info_hash`, or `None` if never attempted.
    pub fn scan_status(&self, info_hash: &[u8; 20]) -> Result<Option<ScannedStatus>> {
        let conn = self.read.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT status, attempts, next_attempt, failure_reason FROM scanned WHERE info_hash = ?1",
                params![info_hash],
                |r| {
                    let status: String = r.get(0)?;
                    let attempts: i64 = r.get(1)?;
                    let next_attempt: i64 = r.get(2)?;
                    let failure_reason: Option<String> = r.get(3)?;
                    Ok((status, attempts, next_attempt, failure_reason))
                },
            )
            .optional()?;
        Ok(row.map(|(status, attempts, next_attempt, failure_reason)| match status.as_str() {
            "ok" => ScannedStatus::Ok,
            "skipped" => ScannedStatus::Skipped,
            _ => ScannedStatus::Failed {
                attempts,
                next_attempt,
                failure_reason,
            },
        }))
    }

    /// True if `info_hash` should not be fetched right now: already accepted,
    /// already filtered out, or a recent failure still inside its backoff
    /// window. `now` is a unix timestamp in seconds.
    ///
    /// Batched production admission uses `scan_blocked_batch`; this helper
    /// keeps the single-hash check available to tests.
    #[cfg(test)]
    pub fn scan_blocked(&self, info_hash: &[u8; 20], now: i64) -> Result<bool> {
        Ok(self.scan_blocked_batch(&[*info_hash], now)?.contains(info_hash))
    }

    /// Batched `scan_blocked`: returns the subset of `info_hashes` that should
    /// NOT be fetched right now (accepted, filtered, or inside a backoff
    /// window). One `IN` query instead of N point lookups keeps pipeline
    /// admission cheap as the unique-hash stream grows. Hashes absent from the
    /// `scanned` table are never blocked.
    pub fn scan_blocked_batch(&self, info_hashes: &[[u8; 20]], now: i64) -> Result<Vec<[u8; 20]>> {
        if info_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", info_hashes.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT info_hash, status, next_attempt FROM scanned WHERE info_hash IN ({placeholders})"
        );
        let conn = self.read.lock().unwrap();
        let mut stmt = conn
            .prepare(&sql)
            .context("prepare batched scan check")?;
        let mut rows = stmt.query_map(
            rusqlite::params_from_iter(info_hashes.iter().map(|h| h.as_slice())),
            |r| {
                let hash_bytes: Vec<u8> = r.get(0)?;
                let status: String = r.get(1)?;
                let next_attempt: i64 = r.get(2)?;
                Ok((hash_bytes, status, next_attempt))
            },
        )?;
        let mut blocked = Vec::new();
        while let Some(Ok((hash_bytes, status, next_attempt))) = rows.next() {
            if hash_bytes.len() != 20 {
                continue;
            }
            let mut h = [0u8; 20];
            h.copy_from_slice(&hash_bytes);
            let is_blocked = match status.as_str() {
                "ok" | "skipped" => true,
                _ => next_attempt > now,
            };
            if is_blocked {
                blocked.push(h);
            }
        }
        Ok(blocked)
    }

    /// Upsert a `scanned` row. A `Failed` record increments the attempt count
    /// and schedules the next attempt with exponential backoff.
    pub fn record_scanned(&self, rec: &ScannedRecord) -> Result<()> {
        let conn = self.write.lock().unwrap();
        match &rec.status {
            ScannedStatus::Ok | ScannedStatus::Skipped => {
                let status = match rec.status {
                    ScannedStatus::Ok => "ok",
                    _ => "skipped",
                };
                conn.execute(
                    "INSERT INTO scanned (info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
                     ON CONFLICT(info_hash) DO UPDATE SET
                         status = excluded.status,
                         info_bytes = excluded.info_bytes,
                         raw_name = excluded.raw_name,
                         attempts = scanned.attempts + 1,
                         last_attempt = excluded.last_attempt,
                         next_attempt = excluded.last_attempt",
                    params![
                        rec.info_hash,
                        status,
                        rec.info_bytes,
                        rec.raw_name,
                        rec.last_attempt,
                    ],
                )?;
            }
            ScannedStatus::Failed { attempts, next_attempt, failure_reason } => {
                conn.execute(
                    "INSERT INTO scanned (info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt, failure_reason)
                     VALUES (?1, 'failed', NULL, NULL, ?2, ?3, ?4, ?5)
                     ON CONFLICT(info_hash) DO UPDATE SET
                         status = 'failed',
                         attempts = ?2,
                         last_attempt = ?3,
                         next_attempt = ?4,
                         failure_reason = ?5",
                    params![rec.info_hash, attempts, rec.last_attempt, next_attempt, failure_reason],
                )?;
            }
        }
        Ok(())
    }

    /// Case-insensitive substring search over the raw release name.
    pub fn search(&self, query: &str) -> Result<Vec<TorrentRecord>> {
        let escaped = query.replace('%', r"\%").replace('_', r"\_");
        let pattern = format!("%{escaped}%");
        let conn = self.read.lock().unwrap();
        let mut stmt = conn.prepare(
            &format!(
                "SELECT {SELECT_COLS} FROM torrents WHERE name LIKE ?1 ESCAPE '\\' ORDER BY last_seen DESC LIMIT 200"
            ),
        )?;
        let rows = stmt.query_map([pattern], Self::row_to_record)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Aggregate metadata fetch failures by dominant reason from the `scanned`
    /// table. Returns `(reason, count)` sorted descending by count.
    pub fn failure_breakdown(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.read.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(failure_reason, 'unknown'), COUNT(*) AS n
             FROM scanned WHERE status = 'failed'
             GROUP BY COALESCE(failure_reason, 'unknown')
             ORDER BY n DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<TorrentRecord> {
        let info_hash: Vec<u8> = row.get(0)?;
        let mut ih = [0u8; 20];
        ih.copy_from_slice(&info_hash);
        Ok(TorrentRecord {
            info_hash: ih,
            name: row.get(1)?,
            size_bytes: row.get(2)?,
            file_count: row.get(3)?,
            first_seen: row.get(4)?,
            last_seen: row.get(5)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(name: &str) -> Storage {
        let path = std::env::temp_dir().join(format!("dht_crawler_{name}_{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        Storage::open(path.to_str().unwrap()).unwrap()
    }

    fn record(hash: u8, first: i64, last: i64) -> TorrentRecord {
        TorrentRecord {
            info_hash: [hash; 20],
            name: "The Matrix 1999".into(),
            size_bytes: Some(1024),
            file_count: Some(1),
            first_seen: first,
            last_seen: last,
        }
    }

    #[test]
    fn upsert_preserves_first_seen() {
        let db = tmp_db("upsert");
        db.insert_batch(&[record(1, 100, 100)]).unwrap();
        db.insert_batch(&[record(1, 9999, 200)]).unwrap();

        let rows = db.search("matrix").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].first_seen, 100, "first_seen must be immutable");
        assert_eq!(rows[0].last_seen, 200);
        assert_eq!(rows[0].size_bytes, Some(1024), "mutable fields are refreshed");
    }

    #[test]
    fn search_match_and_no_match() {
        let db = tmp_db("search");
        db.insert_batch(&[record(3, 1, 1)]).unwrap();

        let hits = db.search("maTrIx").unwrap();
        assert_eq!(hits.len(), 1, "search must be case-insensitive");

        let misses = db.search("nonexistent").unwrap();
        assert!(misses.is_empty());
    }

    #[test]
    fn scanned_records_and_blocking() {
        let db = tmp_db("scanned");
        let hash = [6u8; 20];
        let now = 1_700_000_000i64;

        assert!(!db.scan_blocked(&hash, now).unwrap());

        // Failed: blocked inside the backoff window, allowed after it.
        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + backoff_secs(1),
                failure_reason: Some("timeout".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .unwrap();
        assert!(db.scan_blocked(&hash, now).unwrap());
        assert!(!db.scan_blocked(&hash, now + backoff_secs(1) + 1).unwrap());

        // Ok: permanently blocked.
        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Ok,
            info_bytes: Some(vec![1, 2, 3]),
            raw_name: Some("x".into()),
            last_attempt: now,
        })
        .unwrap();
        assert_eq!(db.scan_status(&hash).unwrap(), Some(ScannedStatus::Ok));
        assert!(db.scan_blocked(&hash, now).unwrap());
        assert!(db.scan_blocked(&hash, now + 100_000).unwrap());

        // Skipped: permanently blocked too.
        let h2 = [7u8; 20];
        db.record_scanned(&ScannedRecord {
            info_hash: h2,
            status: ScannedStatus::Skipped,
            info_bytes: Some(vec![9]),
            raw_name: None,
            last_attempt: now,
        })
        .unwrap();
        assert_eq!(db.scan_status(&h2).unwrap(), Some(ScannedStatus::Skipped));
        assert!(db.scan_blocked(&h2, now).unwrap());
    }

    #[test]
    fn scan_blocked_batch_flags_only_blocked() {
        let db = tmp_db("scan_batch");
        let now = 1_700_000_000i64;
        let accepted = [1u8; 20];
        let in_backoff = [2u8; 20];
        let fresh = [3u8; 20];

        db.record_scanned(&ScannedRecord {
            info_hash: accepted,
            status: ScannedStatus::Ok,
            info_bytes: Some(vec![1]),
            raw_name: None,
            last_attempt: now,
        })
        .unwrap();
        db.record_scanned(&ScannedRecord {
            info_hash: in_backoff,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + 60,
                failure_reason: Some("timeout".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .unwrap();

        let blocked = db
            .scan_blocked_batch(&[accepted, in_backoff, fresh], now)
            .unwrap();
        assert_eq!(blocked.len(), 2, "accepted + backoff blocked, fresh not");
        assert!(blocked.contains(&accepted));
        assert!(blocked.contains(&in_backoff));
        assert!(!blocked.contains(&fresh));

        // Empty input is a no-op.
        assert!(db.scan_blocked_batch(&[], now).unwrap().is_empty());
    }

    #[test]
    fn scanned_failure_increments_attempts() {
        let db = tmp_db("scanned_fail");
        let hash = [8u8; 20];
        let now = 1_700_000_000i64;        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 1,
                next_attempt: now + backoff_secs(1),
                failure_reason: Some("connect_refused".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now,
        })
        .unwrap();
        db.record_scanned(&ScannedRecord {
            info_hash: hash,
            status: ScannedStatus::Failed {
                attempts: 2,
                next_attempt: now + backoff_secs(2),
                failure_reason: Some("no_ut_metadata".into()),
            },
            info_bytes: None,
            raw_name: None,
            last_attempt: now + 600,
        })
        .unwrap();
        assert_eq!(
            db.scan_status(&hash).unwrap(),
            Some(ScannedStatus::Failed {
                attempts: 2,
                next_attempt: now + backoff_secs(2),
                failure_reason: Some("no_ut_metadata".into()),
            })
        );
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(3), 240);
        assert!(backoff_secs(100) <= 6 * 3600);
        assert_eq!(backoff_secs(100), backoff_secs(200));
    }

    #[test]
    fn media_schema_migrates_to_metadata_only() {
        // Simulate a media-era database (with category/title/year/etc.), then
        // open it and confirm the rebuild drops those columns and keeps rows.
        let path = std::env::temp_dir().join(format!(
            "dht_crawler_migrate_meta_{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE torrents (
                info_hash  BLOB PRIMARY KEY,
                name       TEXT NOT NULL,
                category   TEXT NOT NULL CHECK(category IN ('movie', 'tv', 'other')),
                title      TEXT,
                year       INTEGER,
                season     INTEGER,
                episode    INTEGER,
                size_bytes INTEGER,
                file_count INTEGER,
                first_seen INTEGER NOT NULL,
                last_seen  INTEGER NOT NULL
            );
            INSERT INTO torrents VALUES (x'0000000000000000000000000000000000000001', 'Old 1999', 'movie', 'old', 1999, NULL, NULL, 2048, 1, 1, 2);",
        )
        .unwrap();
        drop(conn);

        let storage = Storage::open(path.to_str().unwrap()).unwrap();
        let rows = storage.search("old").unwrap();
        assert_eq!(rows.len(), 1, "old row survives migration");
        assert_eq!(rows[0].size_bytes, Some(2048));

        let conn = storage.read.lock().unwrap();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='torrents'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !sql.contains("category") && !sql.contains("title") && !sql.contains("year"),
            "media columns must be dropped: {sql}"
        );
        drop(conn);

        let _ = std::fs::remove_file(&path);
    }
}
