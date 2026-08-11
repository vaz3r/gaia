use anyhow::Result;
use rusqlite::Connection;

/// Torrent-metadata-only schema. Classification and other enrichment belong in
/// a future `torrent_details` table; the raw info dictionary is retained in
/// `scanned.info_bytes` so it can be re-derived offline.
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS torrents (
    info_hash  BLOB PRIMARY KEY,
    name       TEXT NOT NULL,
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

/// Apply WAL pragmas, create tables, and run idempotent migrations for
/// databases created by earlier schema versions.
pub fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)?;

    // Migration: add failure_reason column to existing databases.
    let _ = conn.execute_batch("ALTER TABLE scanned ADD COLUMN failure_reason TEXT");

    // Migration: rebuild the torrents table to a torrent-metadata-only schema.
    // SQLite cannot drop columns, so rebuild: copy the retained columns from
    // the media-era schema (info_hash, name, size_bytes, file_count,
    // first_seen, last_seen) and drop category/title/year/season/episode.
    if let Ok(sql) = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='torrents'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        if sql.contains("category") {
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE torrents RENAME TO torrents_old;
                 CREATE TABLE torrents (
                     info_hash  BLOB PRIMARY KEY,
                     name       TEXT NOT NULL,
                     size_bytes INTEGER,
                     file_count INTEGER,
                     first_seen INTEGER NOT NULL,
                     last_seen  INTEGER NOT NULL
                 );
                 INSERT INTO torrents (info_hash, name, size_bytes, file_count, first_seen, last_seen)
                     SELECT info_hash, name, size_bytes, file_count, first_seen, last_seen FROM torrents_old;
                 DROP TABLE torrents_old;
                 COMMIT;",
            )?;
        }
    }

    Ok(())
}
