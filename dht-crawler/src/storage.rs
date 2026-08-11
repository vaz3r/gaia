use std::sync::Mutex;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, params, OptionalExtension};
use rusqlite::types::Type;

/// Categories persisted for indexed torrents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Movie,
    Tv,
    /// Everything that is not confidently movie or TV (kept, not filtered).
    Other,
}

impl Category {
    fn as_str(self) -> &'static str {
        match self {
            Category::Movie => "movie",
            Category::Tv => "tv",
            Category::Other => "other",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Category::Movie),
            "tv" => Some(Category::Tv),
            "other" => Some(Category::Other),
            _ => None,
        }
    }
}

/// Exponential backoff in seconds: 5m, 10m, 20m, ... capped at 6h.
pub fn backoff_secs(attempts: i64) -> i64 {
    const BASE: i64 = 300;
    const MAX: i64 = 6 * 3600;
    let n = attempts.max(1) - 1;
    let shift = n.min(30);
    let secs = BASE.saturating_mul(1i64 << shift);
    secs.min(MAX)
}

/// A single accepted torrent record, keyed by its 20-byte info hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentRecord {
    pub info_hash: [u8; 20],
    pub name: String,
    pub category: Category,
    pub title: String,
    pub year: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub size_bytes: Option<i64>,
    pub file_count: Option<i64>,
    pub first_seen: i64,
    pub last_seen: i64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS torrents (
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
CREATE TABLE IF NOT EXISTS scanned (
    info_hash      BLOB PRIMARY KEY,
    status         TEXT NOT NULL CHECK(status IN ('ok', 'skipped', 'failed')),
    info_bytes     BLOB,
    raw_name       TEXT,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_attempt   INTEGER NOT NULL,
    next_attempt   INTEGER NOT NULL,
    failure_reason TEXT
);
";

const UPSERT: &str = "
INSERT INTO torrents (info_hash, name, category, title, year, season, episode, size_bytes, file_count, first_seen, last_seen)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
ON CONFLICT(info_hash) DO UPDATE SET
    name       = excluded.name,
    category   = excluded.category,
    title      = excluded.title,
    year       = excluded.year,
    season     = excluded.season,
    episode    = excluded.episode,
    size_bytes = excluded.size_bytes,
    file_count = excluded.file_count,
    last_seen  = excluded.last_seen
";

const SELECT_COLS: &str = "info_hash, name, category, title, year, season, episode, size_bytes, file_count, first_seen, last_seen";

/// Outcome of a metadata fetch attempt for an infohash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannedStatus {
    /// Metadata fetched, SHA-1 verified, and accepted (movie/TV).
    Ok,
    /// Metadata fetched and verified but filtered out (not movie/TV).
    Skipped,
    /// Metadata could not be fetched; `attempts` and `next_attempt` drive
    /// exponential backoff. `failure_reason` is the dominant failure class.
    Failed {
        attempts: i64,
        next_attempt: i64,
        failure_reason: Option<String>,
    },
}

/// A row in the `scanned` table. `info_bytes` holds the raw bencoded `info`
/// dictionary for `Ok`/`Skipped` rows so classification can be re-run offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedRecord {
    pub info_hash: [u8; 20],
    pub status: ScannedStatus,
    pub info_bytes: Option<Vec<u8>>,
    pub raw_name: Option<String>,
    pub last_attempt: i64,
}

/// SQLite-backed storage with WAL mode. A writer connection handles batched
/// upserts; a reader connection serves membership checks and searches so reads
/// never block the write path.
#[derive(Clone)]
pub struct Storage {
    write: Arc<Mutex<Connection>>,
    read: Arc<Mutex<Connection>>,
}

impl Storage {
    /// Open (or create) the database and initialize the schema.
    pub fn open(path: &str) -> Result<Self> {
        let write = Connection::open(path).with_context(|| format!("open db {path}"))?;
        Self::configure(&write)?;

        let read = Connection::open(path).with_context(|| format!("open db {path}"))?;
        Self::configure(&read)?;

        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read: Arc::new(Mutex::new(read)),
        })
    }

    fn configure(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        // Migration: add failure_reason column to existing databases.
        let _ = conn.execute_batch(
            "ALTER TABLE scanned ADD COLUMN failure_reason TEXT",
        );
        // Migration: widen the category CHECK to include 'other'. SQLite cannot
        // alter a CHECK, so rebuild the table. No-op for fresh databases whose
        // CREATE already includes 'other'.
        if let Ok(sql) = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='torrents'",
            [],
            |r| r.get::<_, String>(0),
        ) {
            if sql.contains("'movie', 'tv')") {
                conn.execute_batch(
                    "BEGIN;
                     ALTER TABLE torrents RENAME TO torrents_old;
                     CREATE TABLE torrents (
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
                     INSERT INTO torrents (info_hash, name, category, title, year, season, episode, size_bytes, file_count, first_seen, last_seen)
                         SELECT info_hash, name, category, title, year, season, episode, size_bytes, file_count, first_seen, last_seen FROM torrents_old;
                     DROP TABLE torrents_old;
                     COMMIT;",
                )?;
            }
        }
        Ok(())
    }

    /// Insert or update a batch of records in a single transaction, preserving
    /// the original `first_seen` on duplicates.
    pub fn insert_batch(&self, records: &[TorrentRecord]) -> Result<()> {
        let mut conn = self.write.lock().unwrap();
        let tx = conn.transaction()?;
        for r in records {
            Self::validate_record(r)?;
            tx.execute(
                UPSERT,
                params![
                    r.info_hash,
                    r.name,
                    r.category.as_str(),
                    r.title,
                    r.year,
                    r.season,
                    r.episode,
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

    fn validate_record(r: &TorrentRecord) -> Result<()> {
        match r.category {
            Category::Movie | Category::Tv | Category::Other => Ok(()),
        }
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
    pub fn scan_blocked(&self, info_hash: &[u8; 20], now: i64) -> Result<bool> {
        Ok(match self.scan_status(info_hash)? {
            None => false,
            Some(ScannedStatus::Ok) | Some(ScannedStatus::Skipped) => true,
            Some(ScannedStatus::Failed { next_attempt, .. }) => next_attempt > now,
        })
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
    pub fn search(&self, query: &str) -> Result<Vec<TorrentRecord>> {        let escaped = query.replace('%', r"\%").replace('_', r"\_");
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

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<TorrentRecord> {        let info_hash: Vec<u8> = row.get(0)?;
        let category: String = row.get(2)?;
        let category = Category::parse(&category)
            .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(std::io::Error::other("bad category"))))?;
        let mut ih = [0u8; 20];
        ih.copy_from_slice(&info_hash);
        Ok(TorrentRecord {
            info_hash: ih,
            name: row.get(1)?,
            category,
            title: row.get(3)?,
            year: row.get(4)?,
            season: row.get(5)?,
            episode: row.get(6)?,
            size_bytes: row.get(7)?,
            file_count: row.get(8)?,
            first_seen: row.get(9)?,
            last_seen: row.get(10)?,
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
            category: Category::Movie,
            title: "the matrix".into(),
            year: Some(1999),
            season: None,
            episode: None,
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
        assert_eq!(rows[0].year, Some(1999), "mutable fields are refreshed");
    }

    #[test]
    fn invalid_category_rejected() {
        let db = tmp_db("badcat");
        let conn = db.write.lock().unwrap();
        // The CHECK constraint must reject unknown categories.
        let res = conn.execute(
            "INSERT INTO torrents (info_hash, name, category, first_seen, last_seen) VALUES (?1, 'x', 'software', 1, 1)",
            params![vec![2u8; 20]],
        );
        assert!(res.is_err(), "CHECK(category IN ('movie','tv','other')) must reject 'software'");
    }

    #[test]
    fn category_parse_rejects_unknown() {
        assert_eq!(Category::parse("movie"), Some(Category::Movie));
        assert_eq!(Category::parse("tv"), Some(Category::Tv));
        assert_eq!(Category::parse("other"), Some(Category::Other));
        assert_eq!(Category::parse("software"), None);
    }

    #[test]
    fn other_category_persisted() {
        let db = tmp_db("other_cat");
        let mut r = record(9, 1, 1);
        r.category = Category::Other;
        db.insert_batch(&[r]).unwrap();
        let rows = db.search("matrix").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, Category::Other);
    }

    #[test]
    fn old_schema_migrates_category_check() {
        // Simulate a pre-'other' database, then run configure() (open) which
        // should rebuild the torrents table with the widened CHECK.
        let path = std::env::temp_dir().join(format!(
            "dht_crawler_migrate_cat_{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE torrents (
                info_hash  BLOB PRIMARY KEY,
                name       TEXT NOT NULL,
                category   TEXT NOT NULL CHECK(category IN ('movie', 'tv')),
                title      TEXT,
                year       INTEGER,
                season     INTEGER,
                episode    INTEGER,
                size_bytes INTEGER,
                file_count INTEGER,
                first_seen INTEGER NOT NULL,
                last_seen  INTEGER NOT NULL
            );
            INSERT INTO torrents VALUES (x'0000000000000000000000000000000000000001', 'Old 1999', 'movie', 'old', 1999, NULL, NULL, NULL, NULL, 1, 1);",
        )
        .unwrap();
        drop(conn);

        let storage = Storage::open(path.to_str().unwrap()).unwrap();
        let rows = storage.search("old").unwrap();
        assert_eq!(rows.len(), 1, "old row survives migration");

        let conn = storage.read.lock().unwrap();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='torrents'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("'other'"),
            "migrated CHECK must include 'other': {sql}"
        );
        drop(conn);

        // 'other' now inserts fine.
        let mut r = record(9, 1, 1);
        r.category = Category::Other;
        storage.insert_batch(&[r]).unwrap();
        let _ = std::fs::remove_file(&path);
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
    fn scanned_failure_increments_attempts() {
        let db = tmp_db("scanned_fail");
        let hash = [8u8; 20];
        let now = 1_700_000_000i64;
        db.record_scanned(&ScannedRecord {
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
        assert_eq!(backoff_secs(1), 300);
        assert_eq!(backoff_secs(2), 600);
        assert_eq!(backoff_secs(3), 1200);
        assert!(backoff_secs(100) <= 6 * 3600);
        assert_eq!(backoff_secs(100), backoff_secs(200));
    }
}
