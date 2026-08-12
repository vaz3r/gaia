use std::io::Write;

use anyhow::{Context, Result};

use crate::cli::PurgeArgs;

/// Delete the database (and its WAL/SHM sidecars) and the routing state
/// directory so a subsequent `run` starts from scratch.
pub fn purge(args: &PurgeArgs) -> Result<()> {
    let targets = [
        args.db.clone(),
        format!("{}-wal", args.db),
        format!("{}-shm", args.db),
    ];

    println!("Purging crawl data:");
    let mut removed = Vec::new();
    for t in &targets {
        let p = std::path::Path::new(t);
        if p.exists() {
            removed.push(t.clone());
        }
    }
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
        eprint!("Delete these files and the routing state? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    for t in &targets {
        let p = std::path::Path::new(t);
        match p.metadata() {
            Ok(_) => {
                std::fs::remove_file(p).with_context(|| format!("remove {}", p.display()))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", p.display())),
        }
    }
    if args.state_dir.exists() {
        std::fs::remove_dir_all(&args.state_dir)
            .with_context(|| format!("remove {}", args.state_dir.display()))?;
    }

    println!("purged");
    Ok(())
}
