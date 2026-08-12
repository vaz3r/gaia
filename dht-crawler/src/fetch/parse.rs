use anyhow::{Context, Result};
use serde::Deserialize;

/// Parsed torrent metadata extracted from a verified info dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMetadata {
    /// Raw release name from the torrent's `name` field.
    pub name: String,
    /// Per-file paths and sizes; empty for single-file torrents.
    pub files: Vec<(String, i64)>,
    /// Total size in bytes (sum of files, or single-file length).
    pub total_size: i64,
    /// Number of files (1 for single-file torrents).
    pub file_count: i64,
}

#[derive(Debug, Deserialize)]
struct FileEntry {
    #[serde(default)]
    length: Option<i64>,
    #[serde(default)]
    path: Vec<serde_bytes::ByteBuf>,
}

#[derive(Debug, Deserialize)]
struct InfoDict {
    name: serde_bytes::ByteBuf,
    #[serde(default)]
    files: Option<Vec<FileEntry>>,
    #[serde(default)]
    length: Option<i64>,
}

/// Parse a verified bencoded info dictionary into extractable fields.
///
/// Unknown fields are tolerated; a missing `name` is treated as an empty
/// string rather than a hard failure.
pub fn extract_metadata(info_bytes: &[u8]) -> Result<ExtractedMetadata> {
    let dict: InfoDict = gaia_bencode::from_bytes_lenient(info_bytes)
        .context("parse bencoded info dictionary")?;

    let name = String::from_utf8_lossy(&dict.name).into_owned();

    let mut files = Vec::new();
    let mut total_size = 0i64;
    let mut file_count = 1i64;

    match dict.files {
        Some(entries) => {
            file_count = entries.len() as i64;
            for entry in entries {
                let len = entry.length.unwrap_or(0);
                let path = entry
                    .path
                    .iter()
                    .map(|p| String::from_utf8_lossy(p).into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                total_size += len;
                files.push((path, len));
            }
        }
        None => {
            total_size = dict.length.unwrap_or(0);
        }
    }

    Ok(ExtractedMetadata {
        name,
        files,
        total_size,
        file_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaia_bencode::to_bytes;
    use serde::Serialize;

    #[derive(Serialize)]
    struct SingleFileInfo {
        name: String,
        length: i64,
        #[serde(rename = "piece length")]
        piece_length: i64,
        pieces: String,
    }

    #[test]
    fn extract_single_file() {
        let info = to_bytes(&SingleFileInfo {
            name: "The Matrix 1999".into(),
            length: 4096,
            piece_length: 16384,
            pieces: "abcdef".into(),
        })
        .unwrap();

        let meta = extract_metadata(&info).unwrap();
        assert_eq!(meta.name, "The Matrix 1999");
        assert_eq!(meta.total_size, 4096);
        assert_eq!(meta.file_count, 1);
        assert!(meta.files.is_empty());
    }

    #[test]
    fn extract_multi_file() {
        #[derive(Serialize)]
        struct MultiFileInfo<'a> {
            name: &'a str,
            files: Vec<FileEntrySer>,
        }
        #[derive(Serialize)]
        struct FileEntrySer {
            length: i64,
            path: Vec<String>,
        }

        let info = to_bytes(&MultiFileInfo {
            name: "Show S01",
            files: vec![
                FileEntrySer {
                    length: 10,
                    path: vec!["Show".into(), "a.mkv".into()],
                },
                FileEntrySer {
                    length: 20,
                    path: vec!["Show".into(), "b.mkv".into()],
                },
            ],
        })
        .unwrap();

        let meta = extract_metadata(&info).unwrap();
        assert_eq!(meta.name, "Show S01");
        assert_eq!(meta.total_size, 30);
        assert_eq!(meta.file_count, 2);
        assert_eq!(meta.files.len(), 2);
        assert_eq!(meta.files[1], ("Show/b.mkv".to_string(), 20));
    }

    #[test]
    fn extract_tolerates_missing_length() {
        #[derive(Serialize)]
        struct Minimal {
            name: String,
        }
        let info = to_bytes(&Minimal {
            name: "no-length".into(),
        })
        .unwrap();
        let meta = extract_metadata(&info).unwrap();
        assert_eq!(meta.total_size, 0);
        assert_eq!(meta.file_count, 1);
    }
}
