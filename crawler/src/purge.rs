use std::io::Write;

use anyhow::{Context, Result};
use redis::AsyncCommands;
use sqlx::PgPool;

use crate::cli::PurgeArgs;

/// Delete the crawl data: TRUNCATE Postgres tables and flush the Redis state
/// (node pool, per-instance node IDs, sampler state) so a subsequent `run`
/// starts from scratch. There is no file persistence anymore.
pub async fn purge(args: &PurgeArgs) -> Result<()> {
    if !args.yes {
        eprint!("Purge Postgres crawl data and the Redis DHT state? [y/N] ");
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
    println!("  - postgres: torrents, scanned, crawl_stats_history");

    // Flush the Redis DHT state keys (best-effort; no-op without redis).
    if let Some(redis_url) = &args.redis_url {
        let client = redis::Client::open(redis_url.as_str())
            .with_context(|| format!("open redis {redis_url}"))?;
        let mut conn = redis::aio::ConnectionManager::new(client)
            .await
            .with_context(|| format!("connect redis {redis_url}"))?;
        let keys = ["nodes", "samp:interval", "samp:quality"];
        for k in keys {
            let _: redis::RedisResult<i64> = conn.del(format!("dht:{k}")).await;
        }
        // Per-instance node IDs: dht:node:{0..N}.
        for i in 0..32 {
            let _: redis::RedisResult<i64> = conn.del(format!("dht:node:{i}")).await;
        }
        println!("  - redis: dht:nodes, dht:samp:* , dht:node:*");
    }

    println!("purged");
    Ok(())
}
