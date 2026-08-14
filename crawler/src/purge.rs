use std::io::Write;

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::cli::PurgeArgs;

/// Delete the crawl data (TRUNCATE Postgres tables) and the routing state
/// directory so a subsequent `run` starts from scratch.
pub async fn purge(args: &PurgeArgs) -> Result<()> {
    println!("Purging crawl data:");
    let mut removed = Vec::new();
    if args.state_dir.exists() {
        removed.push(args.state_dir.display().to_string());
    }

    if removed.is_empty() {
        println!("  nothing to purge");
        return Ok(());
    }
    for r in &removed {
        println!("  - {r}");
    }

    if !args.yes {
        eprint!("Delete the crawl tables and the routing state? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    let pool = PgPool::connect(&args.pg)
        .await
        .with_context(|| format!("connect postgres {}", args.pg))?;
    sqlx::query("TRUNCATE TABLE torrents, scanned, crawl_stats_history")
        .execute(&pool)
        .await
        .context("truncate crawl tables")?;

    if args.state_dir.exists() {
        std::fs::remove_dir_all(&args.state_dir)
            .with_context(|| format!("remove {}", args.state_dir.display()))?;
    }

    println!("purged");
    Ok(())
}
