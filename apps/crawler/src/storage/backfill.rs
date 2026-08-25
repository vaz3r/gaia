use crate::storage::torrents::parse_info_dict;
use sqlx::PgPool;
use std::io::Read;
use std::path::Path;

pub async fn run(pool: &PgPool, data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let meta_path = data_dir.join("metadata.bin");
    let discovered_path = data_dir.join("discovered.txt");

    if meta_path.exists() {
        let n = import_metadata_bin(pool, &meta_path).await?;
        tracing::info!(torrents = n, "backfill: metadata.bin imported");
    } else {
        tracing::info!("backfill: no metadata.bin found");
    }

    if discovered_path.exists() {
        let n = import_discovered(pool, &discovered_path).await?;
        tracing::info!(sightings = n, "backfill: discovered.txt imported");
    } else {
        tracing::info!("backfill: no discovered.txt found");
    }

    Ok(())
}

async fn import_metadata_bin(
    pool: &PgPool,
    path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut header = [0u8; 24];
    let mut total = 0usize;
    loop {
        let mut read = 0usize;
        while read < 24 {
            let n = reader.read(&mut header[read..])?;
            if n == 0 {
                return Ok(total);
            }
            read += n;
        }
        let mut ih = [0u8; 20];
        ih.copy_from_slice(&header[..20]);
        let len = u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let mut meta = vec![0u8; len];
        reader.read_exact(&mut meta)?;

        let mut tx = pool.begin().await?;
        let json = r#"{"legacy":1}"#;
        sqlx::query(
            "INSERT INTO infohash_sightings (infohash, source_counts) VALUES ($1, $2::jsonb) \
             ON CONFLICT (infohash) DO UPDATE SET last_seen = now(), \
             source_counts = infohash_sightings.source_counts || $2::jsonb",
        )
        .bind(ih.as_slice())
        .bind(json)
        .execute(&mut *tx)
        .await?;
        let p = parse_info_dict(&meta);
        let files = p.files.as_ref().map(serde_json::Value::to_string);
        sqlx::query(
            "INSERT INTO torrents (infohash, name, piece_length, total_size, file_count, files, verified_at) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, now()) \
             ON CONFLICT (infohash) DO UPDATE SET \
             name = EXCLUDED.name, piece_length = EXCLUDED.piece_length, \
             total_size = EXCLUDED.total_size, file_count = EXCLUDED.file_count, files = EXCLUDED.files",
        )
        .bind(ih.as_slice())
        .bind(p.name.as_deref())
        .bind(p.piece_length)
        .bind(p.total_size)
        .bind(p.file_count)
        .bind(files.as_deref())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO verification_jobs (infohash, status, updated_at) VALUES ($1, 'verified', now()) \
             ON CONFLICT (infohash) DO NOTHING",
        )
        .bind(ih.as_slice())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        total += 1;
        if total.is_multiple_of(1000) {
            tracing::info!(processed = total, "backfill: metadata.bin progress");
        }
    }
}

async fn import_discovered(
    pool: &PgPool,
    path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let data = std::fs::read_to_string(path)?;
    let json = r#"{"legacy":1}"#;
    let mut total = 0usize;
    let mut tx = pool.begin().await?;
    for line in data.lines() {
        let line = line.trim();
        if line.len() != 40 {
            continue;
        }
        let mut ih = [0u8; 20];
        let ok = (0..20).all(|i| match u8::from_str_radix(&line[i * 2..i * 2 + 2], 16) {
            Ok(hi) => {
                ih[i] = hi;
                true
            }
            Err(_) => false,
        });
        if !ok {
            continue;
        }
        sqlx::query(
            "INSERT INTO infohash_sightings (infohash, source_counts) VALUES ($1, $2::jsonb) \
             ON CONFLICT (infohash) DO UPDATE SET last_seen = now(), \
             source_counts = infohash_sightings.source_counts || $2::jsonb",
        )
        .bind(ih.as_slice())
        .bind(json)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO verification_jobs (infohash, status, next_retry_at, updated_at) \
             VALUES ($1, 'pending', now(), now()) ON CONFLICT (infohash) DO NOTHING",
        )
        .bind(ih.as_slice())
        .execute(&mut *tx)
        .await?;
        total += 1;
        if total.is_multiple_of(20000) {
            tx.commit().await?;
            tracing::info!(processed = total, "backfill: discovered.txt progress");
            tx = pool.begin().await?;
        }
    }
    tx.commit().await?;
    Ok(total)
}
