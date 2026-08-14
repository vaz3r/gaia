//! One-shot SQLite → PostgreSQL migration for the crawler.
//!
//! Moved out of the crawler binary so the crawler can drop rusqlite at runtime
//! (and use sqlx's `migrate` feature without the sqlite3 link conflict). Run
//! once against the pre-Postgres DB before the crawler cuts over to Postgres:
//!
//! ```sh
//! cargo run --release -p sqlite-to-pg -- --sqlite <path> --pg <url> [--batch 50000]
//! ```

use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use rusqlite::Connection;
use sqlx::query_builder::QueryBuilder;
use sqlx::{PgPool, Row};

const MAX_BIND_PARAMS: usize = 60_000;

fn clamped_batch(requested: usize, columns: usize) -> usize {
    (requested.min(MAX_BIND_PARAMS / columns.max(1))).max(1)
}

#[derive(Debug, Parser)]
#[command(name = "sqlite-to-pg", about = "Copy crawler torrents/scanned from SQLite into Postgres")]
struct Args {
    /// SQLite database file to read from.
    #[arg(long, default_value = "crawler.sqlite")]
    sqlite: String,

    /// PostgreSQL connection URL.
    #[arg(long)]
    pg: String,

    /// Rows per batch (auto-clamped to Postgres' 65535 bind-param limit).
    #[arg(long, default_value_t = 50_000)]
    batch: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    migrate(&args).await
}

async fn copy_torrents(sqlite: &Connection, pg: &PgPool, batch: usize) -> Result<u64> {
    let batch = clamped_batch(batch, 6);
    let mut stmt = sqlite
        .prepare(
            "SELECT info_hash, name, size_bytes, file_count, first_seen, last_seen FROM torrents",
        )
        .context("prepare sqlite torrents select")?;
    let mut rows = stmt.query([]).context("query sqlite torrents")?;

    let mut total: u64 = 0;
    loop {
        let mut chunk = Vec::with_capacity(batch);
        while chunk.len() < batch {
            match rows.next()? {
                Some(r) => {
                    chunk.push((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                    ));
                }
                None => break,
            }
        }
        if chunk.is_empty() {
            break;
        }

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO torrents (info_hash, name, size_bytes, file_count, first_seen, last_seen) ",
        );
        qb.push_values(chunk.iter(), |mut b, row| {
            b.push_bind(row.0.as_slice())
                .push_bind(row.1.as_str())
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5);
        });
        qb.push(" ON CONFLICT (info_hash) DO NOTHING");

        qb.build().execute(pg).await.context("insert torrents batch")?;
        total += chunk.len() as u64;
        tracing::info!(table = "torrents", copied = total, "batch committed");
    }
    Ok(total)
}

async fn copy_scanned(sqlite: &Connection, pg: &PgPool, batch: usize) -> Result<u64> {
    let batch = clamped_batch(batch, 8);
    let mut stmt = sqlite
        .prepare(
            "SELECT info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt, failure_reason FROM scanned",
        )
        .context("prepare sqlite scanned select")?;
    let mut rows = stmt.query([]).context("query sqlite scanned")?;

    let mut total: u64 = 0;
    loop {
        let mut chunk = Vec::with_capacity(batch);
        while chunk.len() < batch {
            match rows.next()? {
                Some(r) => {
                    chunk.push((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ));
                }
                None => break,
            }
        }
        if chunk.is_empty() {
            break;
        }

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO scanned (info_hash, status, info_bytes, raw_name, attempts, last_attempt, next_attempt, failure_reason) ",
        );
        qb.push_values(chunk.iter(), |mut b, row| {
            b.push_bind(row.0.as_slice())
                .push_bind(row.1.as_str())
                .push_bind(row.2.as_deref())
                .push_bind(row.3.as_deref())
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7.as_deref());
        });
        qb.push(" ON CONFLICT (info_hash) DO NOTHING");

        qb.build().execute(pg).await.context("insert scanned batch")?;
        total += chunk.len() as u64;
        tracing::info!(table = "scanned", copied = total, "batch committed");
    }
    Ok(total)
}

async fn verify_counts(sqlite: &Connection, pg: &PgPool) -> Result<()> {
    let count = |table: &str, db: &rusqlite::Connection| -> rusqlite::Result<i64> {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
    };

    let (torrents_src, scanned_src) = (count("torrents", sqlite)?, count("scanned", sqlite)?);
    let (torrents_dst, scanned_dst): (i64, i64) = {
        let row = sqlx::query(
            "SELECT (SELECT COUNT(*) FROM torrents)::int8, (SELECT COUNT(*) FROM scanned)::int8",
        )
        .fetch_one(pg)
        .await
        .context("count destination")?;
        (row.get(0), row.get(1))
    };

    let torrents_ok = torrents_src == torrents_dst;
    let scanned_ok = scanned_src == scanned_dst;

    println!("migration summary:");
    println!(
        "  torrents: source={torrents_src} destination={torrents_dst} {}",
        if torrents_ok { "OK" } else { "MISMATCH" }
    );
    println!(
        "  scanned:  source={scanned_src} destination={scanned_dst} {}",
        if scanned_ok { "OK" } else { "MISMATCH" }
    );

    if torrents_ok && scanned_ok {
        println!("migration successful: all counts match");
    } else {
        println!("migration WARNING: counts do not match");
    }
    Ok(())
}

async fn migrate(args: &Args) -> Result<()> {
    let started = Instant::now();
    let sqlite = Connection::open(&args.sqlite)
        .with_context(|| format!("open sqlite {}", args.sqlite))?;

    let pg = PgPool::connect(&args.pg)
        .await
        .with_context(|| format!("connect postgres {}", args.pg))?;

    tracing::info!(sqlite = %args.sqlite, pg = %args.pg, batch = args.batch, "starting migration");

    let t = copy_torrents(&sqlite, &pg, args.batch).await?;
    tracing::info!(table = "torrents", copied = t, "torrents done");

    let s = copy_scanned(&sqlite, &pg, args.batch).await?;
    tracing::info!(table = "scanned", copied = s, "scanned done");

    verify_counts(&sqlite, &pg).await?;

    tracing::info!(elapsed_ms = started.elapsed().as_millis(), "migration complete");
    Ok(())
}
