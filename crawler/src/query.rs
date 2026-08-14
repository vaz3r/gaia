use anyhow::Result;

use crate::cli::QueryArgs;
use crate::storage::Storage;

/// Search the database and print matching torrent metadata.
pub async fn query(args: QueryArgs) -> Result<()> {
    let storage = Storage::connect(&args.pg).await?;
    if args.failures {
        return query_failures(&storage).await;
    }
    let rows = storage.search(&args.name).await?;
    if rows.is_empty() {
        println!("no matches for {:?}", args.name);
        return Ok(());
    }
    for r in rows {
        let size = r.size_bytes.map_or("-".to_string(), |b| {
            if b >= 1024 * 1024 * 1024 {
                format!("{:.1} GiB", b as f64 / (1024.0 * 1024.0 * 1024.0))
            } else if b >= 1024 * 1024 {
                format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0))
            } else {
                format!("{b} B")
            }
        });
        let files = r.file_count.map_or("-".to_string(), |f| f.to_string());
        println!("{name}\t{size}\t{files} files", name = r.name);
    }
    Ok(())
}

async fn query_failures(storage: &Storage) -> Result<()> {
    let rows = storage.failure_breakdown().await?;
    if rows.is_empty() {
        println!("no failed fetches recorded yet");
        return Ok(());
    }
    println!("failed fetches by dominant reason:");
    for (reason, count) in rows {
        println!("  {count:>8}  {reason}");
    }
    Ok(())
}
