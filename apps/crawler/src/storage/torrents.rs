use crate::krpc::Infohash;
use crate::krpc::codec::{BValue, decode_prefix};
use bytes::Bytes;
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TorrentStore {
    pool: PgPool,
    written: AtomicU64,
}

pub struct ParsedTorrent {
    pub name: Option<String>,
    pub piece_length: Option<i64>,
    pub total_size: Option<i64>,
    pub file_count: Option<i64>,
    pub files: Option<serde_json::Value>,
}

impl ParsedTorrent {
    fn empty() -> Self {
        ParsedTorrent {
            name: None,
            piece_length: None,
            total_size: None,
            file_count: None,
            files: None,
        }
    }
}

impl TorrentStore {
    pub fn new(pool: PgPool) -> Self {
        TorrentStore {
            pool,
            written: AtomicU64::new(0),
        }
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub async fn store(&self, ih: Infohash, metadata: &[u8]) -> Result<(), sqlx::Error> {
        let p = parse_info_dict(metadata);
        let files = p.files.as_ref().map(serde_json::Value::to_string);
        sqlx::query(
            "INSERT INTO torrents (infohash, name, piece_length, total_size, file_count, files, verified_at, \
             health_score, popularity_score, swarm_peers, seed_confirmed, last_health_check) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, now(), 90, 50, 1, true, now()) \
             ON CONFLICT (infohash) DO UPDATE SET \
             name = EXCLUDED.name, piece_length = EXCLUDED.piece_length, \
             total_size = EXCLUDED.total_size, file_count = EXCLUDED.file_count, files = EXCLUDED.files, \
             verified_at = now(), health_score = 90, seed_confirmed = true, last_health_check = now()",
        )
        .bind(ih.as_slice())
        .bind(p.name.as_deref())
        .bind(p.piece_length)
        .bind(p.total_size)
        .bind(p.file_count)
        .bind(files.as_deref())
        .execute(&self.pool)
        .await?;
        self.written.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn sanitize_control(s: String) -> String {
    if s.chars().any(char::is_control) {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    } else {
        s
    }
}

pub fn parse_info_dict(metadata: &[u8]) -> ParsedTorrent {
    let bytes = Bytes::copy_from_slice(metadata);
    let (mut info, _) = match decode_prefix(&bytes) {
        Ok(v) => v,
        Err(_) => return ParsedTorrent::empty(),
    };
    if let Some(inner) = info.get(b"info").and_then(BValue::as_dict) {
        info = BValue::Dict(inner.clone());
    }
    if info.as_dict().is_none() {
        return ParsedTorrent::empty();
    }
    let name = info
        .get_bytes(b"name")
        .map(|b| sanitize_control(String::from_utf8_lossy(b).into_owned()));
    let piece_length = info.get_int(b"piece length");
    let single_len = info.get_int(b"length");
    let (file_count, files, total_size) = match info.get(b"files").and_then(BValue::as_list) {
        Some(list) => {
            let mut count = 0i64;
            let mut total = 0i64;
            let mut arr = Vec::with_capacity(list.len());
            for f in list {
                let fd = match f.as_dict() {
                    Some(fd) => BValue::Dict(fd.clone()),
                    None => continue,
                };
                let len = fd.get_int(b"length").unwrap_or(0).max(0);
                total = total.saturating_add(len);
                count += 1;
                let path = fd
                    .get(b"path")
                    .and_then(BValue::as_list)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(BValue::as_bytes)
                            .map(|b| sanitize_control(String::from_utf8_lossy(b).into_owned()))
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                arr.push(serde_json::json!({ "length": len, "path": path }));
            }
            (
                Some(count),
                Some(serde_json::Value::Array(arr)),
                Some(total),
            )
        }
        None => (Some(1), None, single_len),
    };
    ParsedTorrent {
        name,
        piece_length,
        total_size,
        file_count,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_file_info() {
        let meta =
            b"d6:lengthi42e4:name4:test12:piece lengthi16384e6:pieces20:12345678901234567890e";
        let p = parse_info_dict(meta);
        assert_eq!(p.name.as_deref(), Some("test"));
        assert_eq!(p.piece_length, Some(16384));
        assert_eq!(p.total_size, Some(42));
        assert_eq!(p.file_count, Some(1));
    }

    #[test]
    fn parse_multi_file_info() {
        let meta = b"d5:filesld6:lengthi10e4:pathl4:afileeee12:piece lengthi16384e6:pieces20:12345678901234567890e";
        let p = parse_info_dict(meta);
        assert_eq!(p.file_count, Some(1));
        assert_eq!(p.total_size, Some(10));
        let files = p.files.unwrap();
        assert_eq!(files[0]["length"], 10);
    }

    #[test]
    fn parse_garbage() {
        let p = parse_info_dict(b"not bencode at all");
        assert_eq!(p.name, None);
    }

    #[test]
    fn sanitize_control_chars() {
        assert_eq!(sanitize_control("ab\u{0000}cd".to_string()), "ab cd");
        assert_eq!(sanitize_control("tab\there".to_string()), "tab here");
        assert_eq!(sanitize_control("normal".to_string()), "normal");
    }
}
