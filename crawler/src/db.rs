//! Lightweight runtime migration runner.
//!
//! Applies the SQL migration files from `db/migrations` in filename order,
//! tracking applied ones in a `_migrations` table (name + applied_at). Each
//! migration runs inside a transaction; a partial failure rolls back and is
//! reported, leaving the DB un-migrated so a retry is safe.
//!
//! We avoid sqlx's `migrate` feature here: it pulls sqlx-sqlite, which cannot
//! coexist with rusqlite in the same workspace (both link `sqlite3`).

use anyhow::{Context, Result};
use sqlx::PgPool;

/// Migrations embedded at build time. Keep the file list in sync with
/// `db/migrations/` — the ORDER matters (they are applied in this order).
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "20260814190000_init.sql",
        include_str!("../../db/migrations/20260814190000_init.sql"),
    ),
    (
        "20260814190001_pgvector.sql",
        include_str!("../../db/migrations/20260814190001_pgvector.sql"),
    ),
];

/// Apply all pending migrations. Idempotent: applied migrations are skipped.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            name       TEXT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .execute(pool)
    .await
    .context("create _migrations table")?;

    for (name, sql) in MIGRATIONS {
        let applied: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _migrations WHERE name = $1)")
                .bind(name)
                .fetch_one(pool)
                .await
                .with_context(|| format!("check migration {name}"))?;
        if applied {
            tracing::debug!(migration = name, "already applied, skipping");
            continue;
        }

        let mut tx = pool.begin().await.context("begin migration tx")?;
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("apply migration {name}"))?;
        sqlx::query("INSERT INTO _migrations (name) VALUES ($1)")
            .bind(name)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("record migration {name}"))?;
        tx.commit().await.with_context(|| format!("commit migration {name}"))?;
        tracing::info!(migration = name, "applied");
    }
    Ok(())
}
