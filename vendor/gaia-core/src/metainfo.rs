use bytes::Bytes;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::hash::Id20;

/// Deserialize a list of raw 20-byte binary strings into `Vec<Id20>`, silently
/// dropping entries that are not exactly 20 bytes.
///
/// BEP 38 `similar` is a list of info hashes (raw 20-byte binary).  Rather than
/// rejecting the entire torrent when a single entry has the wrong length, we
/// keep only the valid ones — a robustness-over-strictness choice consistent
/// with Postel's law.
fn deserialize_similar<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Id20>, D::Error> {
    struct SimilarVisitor;

    impl<'de> de::Visitor<'de> for SimilarVisitor {
        type Value = Vec<Id20>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a list of 20-byte binary strings")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Id20>, A::Error> {
            let mut hashes = Vec::new();
            // Each element is a raw byte string; accept via serde_bytes.
            while let Some(bytes) = seq.next_element::<serde_bytes::ByteBuf>()? {
                if let Ok(id) = Id20::from_bytes(bytes.as_ref()) {
                    hashes.push(id);
                }
                // Silently drop entries that are not exactly 20 bytes.
            }
            Ok(hashes)
        }
    }

    deserializer.deserialize_seq(SimilarVisitor)
}

/// Wrapper for `url-list` that handles both a single string and a list of strings.
#[derive(Debug, Clone, Default)]
pub struct UrlList(pub Vec<String>);

impl<'de> Deserialize<'de> for UrlList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UrlListVisitor;

        impl<'de> de::Visitor<'de> for UrlListVisitor {
            type Value = UrlList;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string or list of strings")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<UrlList, E> {
                Ok(UrlList(vec![v.to_owned()]))
            }

            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<UrlList, E> {
                let s = std::str::from_utf8(v).map_err(de::Error::custom)?;
                Ok(UrlList(vec![s.to_owned()]))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<UrlList, A::Error> {
                let mut urls = Vec::new();
                while let Some(url) = seq.next_element::<String>()? {
                    urls.push(url);
                }
                Ok(UrlList(urls))
            }
        }

        deserializer.deserialize_any(UrlListVisitor)
    }
}

/// Parsed .torrent file (BEP 3 metainfo, v1).
#[derive(Debug, Clone)]
pub struct TorrentMetaV1 {
    /// The info hash (SHA1 of the raw "info" dict bytes).
    pub info_hash: Id20,
    /// Primary announce URL.
    pub announce: Option<String>,
    /// Announce list (BEP 12) — list of tracker tiers.
    pub announce_list: Option<Vec<Vec<String>>>,
    /// Comment.
    pub comment: Option<String>,
    /// Created by.
    pub created_by: Option<String>,
    /// Creation date (unix timestamp).
    pub creation_date: Option<i64>,
    /// Info dictionary.
    pub info: InfoDict,
    /// BEP 19 web seed URLs (GetRight-style).
    pub url_list: Vec<String>,
    /// BEP 17 HTTP seed URLs (Hoffman-style).
    pub httpseeds: Vec<String>,
    /// Raw info dict bytes for BEP 9 metadata serving.
    pub info_bytes: Option<Bytes>,
    /// PEM-encoded SSL CA certificate from the info dict, if present.
    pub ssl_cert: Option<Vec<u8>>,
}

impl TorrentMetaV1 {
    /// M254: serialize back to `.torrent` file bytes.
    ///
    /// The raw `info_bytes` are spliced VERBATIM (parsing and
    /// re-serializing would sort dict keys and change the info-hash for
    /// non-canonical torrents in the wild) — hash-exact by construction.
    /// Falls back to serde-serializing [`InfoDict`] (canonical, sorted)
    /// when the raw bytes are absent. The outer dict is assembled
    /// manually in bencode-sorted key order:
    /// `announce` < `announce-list` < `comment` < `created by` <
    /// `creation date` < `httpseeds` < `info` < `url-list`.
    ///
    /// # Errors
    ///
    /// Returns an error if the [`InfoDict`] fallback serialization fails.
    pub fn to_torrent_bytes(&self) -> Result<Vec<u8>, Error> {
        fn push_key(out: &mut Vec<u8>, key: &[u8]) {
            out.extend_from_slice(key.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(key);
        }
        fn push_str(out: &mut Vec<u8>, s: &str) {
            out.extend_from_slice(s.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(s.as_bytes());
        }
        fn push_str_list(out: &mut Vec<u8>, items: &[String]) {
            out.push(b'l');
            for item in items {
                push_str(out, item);
            }
            out.push(b'e');
        }

        let info_raw: Vec<u8> = match &self.info_bytes {
            Some(raw) => raw.to_vec(),
            None => gaia_bencode::to_bytes(&self.info)?,
        };

        let mut out = Vec::with_capacity(info_raw.len() + 256);
        out.push(b'd');
        if let Some(ref announce) = self.announce {
            push_key(&mut out, b"announce");
            push_str(&mut out, announce);
        }
        if let Some(ref tiers) = self.announce_list {
            push_key(&mut out, b"announce-list");
            out.push(b'l');
            for tier in tiers {
                push_str_list(&mut out, tier);
            }
            out.push(b'e');
        }
        if let Some(ref comment) = self.comment {
            push_key(&mut out, b"comment");
            push_str(&mut out, comment);
        }
        if let Some(ref created_by) = self.created_by {
            push_key(&mut out, b"created by");
            push_str(&mut out, created_by);
        }
        if let Some(date) = self.creation_date {
            push_key(&mut out, b"creation date");
            out.push(b'i');
            out.extend_from_slice(date.to_string().as_bytes());
            out.push(b'e');
        }
        if !self.httpseeds.is_empty() {
            push_key(&mut out, b"httpseeds");
            push_str_list(&mut out, &self.httpseeds);
        }
        push_key(&mut out, b"info");
        out.extend_from_slice(&info_raw);
        if !self.url_list.is_empty() {
            push_key(&mut out, b"url-list");
            push_str_list(&mut out, &self.url_list);
        }
        out.push(b'e');
        Ok(out)
    }
}

/// The "info" dictionary from a .torrent file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InfoDict {
    /// Suggested file/directory name.
    pub name: String,
    /// Piece length in bytes.
    #[serde(rename = "piece length")]
    pub piece_length: u64,
    /// Concatenated SHA1 hashes of each piece (20 bytes each).
    #[serde(with = "serde_bytes")]
    pub pieces: Vec<u8>,
    /// Length in bytes (single-file mode).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub length: Option<u64>,
    /// Files (multi-file mode).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub files: Option<Vec<FileEntry>>,
    /// Private flag.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub private: Option<i64>,
    /// Source tag (private tracker identification).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    /// BEP 35 / SSL torrent: PEM-encoded X.509 CA certificate.
    /// When present, all peer connections must use TLS with certs chaining to this CA.
    #[serde(rename = "ssl-cert", skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "serde_bytes")]
    pub ssl_cert: Option<Vec<u8>>,
    /// BEP 38: info hashes of similar/related torrents (raw 20-byte binary strings).
    ///
    /// Entries that are not exactly 20 bytes are silently dropped during parsing.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_similar"
    )]
    pub similar: Vec<Id20>,
    /// BEP 38: collection names this torrent belongs to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<String>,
}

/// A file entry in multi-file mode.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileEntry {
    /// File length in bytes.
    pub length: u64,
    /// Path components (e.g., `["dir", "file.txt"]`).
    pub path: Vec<String>,
    /// BEP 47 file attributes ("p"=pad, "h"=hidden, "x"=executable, "l"=symlink).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attr: Option<String>,
    /// File modification time (unix timestamp).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mtime: Option<i64>,
    /// Symlink target path components.
    #[serde(
        rename = "symlink path",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub symlink_path: Option<Vec<String>>,
}

/// High-level file info (unified from single-file and multi-file modes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Relative path components.
    pub path: Vec<String>,
    /// File length in bytes.
    pub length: u64,
}

/// Raw top-level torrent structure for serde deserialization.
#[derive(Deserialize)]
struct RawTorrent {
    announce: Option<String>,
    #[serde(rename = "announce-list")]
    announce_list: Option<Vec<Vec<String>>>,
    comment: Option<String>,
    #[serde(rename = "created by")]
    created_by: Option<String>,
    #[serde(rename = "creation date")]
    creation_date: Option<i64>,
    info: InfoDict,
    /// BEP 19: web seed URL(s) — single string or list.
    #[serde(rename = "url-list", default)]
    url_list: UrlList,
    /// BEP 17: HTTP seed URLs.
    #[serde(default)]
    httpseeds: Vec<String>,
}

/// Parse a .torrent file from raw bytes.
///
/// Computes the info-hash by finding the raw byte span of the "info" key
/// and SHA1-hashing it directly (not the re-serialized form).
///
/// # Errors
///
/// Returns an error if the data is not a valid v1 torrent file.
pub fn torrent_from_bytes(data: &[u8]) -> Result<TorrentMetaV1, Error> {
    // Step 1: Find the raw info dict span for hashing
    let info_span = gaia_bencode::find_dict_key_span(data, "info")?;
    let info_hash = crate::sha1(&data[info_span.clone()]);
    let info_raw = Bytes::copy_from_slice(&data[info_span]);

    // Step 2: Deserialize the full structure
    let raw: RawTorrent = gaia_bencode::from_bytes(data)?;

    // Step 3: Validate the info dict
    validate_info(&raw.info)?;

    let ssl_cert = raw.info.ssl_cert.clone();

    Ok(TorrentMetaV1 {
        info_hash,
        announce: raw.announce,
        announce_list: raw.announce_list,
        comment: raw.comment,
        created_by: raw.created_by,
        creation_date: raw.creation_date,
        info: raw.info,
        url_list: raw.url_list.0,
        httpseeds: raw.httpseeds,
        info_bytes: Some(info_raw),
        ssl_cert,
    })
}

fn validate_info(info: &InfoDict) -> Result<(), Error> {
    if info.piece_length == 0 {
        return Err(Error::InvalidTorrent("piece length is 0".into()));
    }

    if !info.pieces.len().is_multiple_of(20) {
        return Err(Error::InvalidTorrent(format!(
            "pieces length {} is not a multiple of 20",
            info.pieces.len()
        )));
    }

    if info.length.is_none() && info.files.is_none() {
        return Err(Error::InvalidTorrent(
            "neither 'length' nor 'files' present".into(),
        ));
    }

    if info.length.is_some() && info.files.is_some() {
        return Err(Error::InvalidTorrent(
            "both 'length' and 'files' present".into(),
        ));
    }

    Ok(())
}

impl InfoDict {
    /// Total size of all files in bytes.
    #[must_use]
    pub fn total_length(&self) -> u64 {
        if let Some(length) = self.length {
            length
        } else if let Some(ref files) = self.files {
            files.iter().map(|f| f.length).sum()
        } else {
            0
        }
    }

    /// Number of pieces.
    #[must_use]
    pub fn num_pieces(&self) -> usize {
        self.pieces.len() / 20
    }

    /// Get the SHA1 hash for a specific piece.
    #[must_use]
    pub fn piece_hash(&self, index: usize) -> Option<Id20> {
        let start = index * 20;
        if start + 20 > self.pieces.len() {
            return None;
        }
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&self.pieces[start..start + 20]);
        Some(Id20(hash))
    }

    /// Get file info in a unified format.
    #[must_use]
    pub fn files(&self) -> Vec<FileInfo> {
        if let Some(length) = self.length {
            vec![FileInfo {
                path: vec![self.name.clone()],
                length,
            }]
        } else if let Some(ref files) = self.files {
            files
                .iter()
                .map(|f| {
                    let mut path = vec![self.name.clone()];
                    path.extend(f.path.clone());
                    FileInfo {
                        path,
                        length: f.length,
                    }
                })
                .collect()
        } else {
            vec![]
        }
    }
}

/// How a torrent's file tree is materialized under the download directory
/// (M252/ER5; mirrors qBittorrent's "Content layout").
///
/// Transforms operate on [`InfoDict::files()`] output, whose canonical shape
/// is `[name]` for single-file torrents and `[name, …components]` for
/// multi-file torrents (root always first).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentLayout {
    /// Keep the torrent's own structure exactly as the metainfo describes it.
    #[default]
    Original,
    /// Always create a subfolder named after the torrent — wraps single-file
    /// torrents in `name/`; multi-file torrents already have one (no-op).
    Subfolder,
    /// Never create a subfolder — strips the root component from multi-file
    /// torrents; single-file torrents are already flat (no-op).
    NoSubfolder,
}

impl ContentLayout {
    /// Apply this layout to canonical [`InfoDict::files()`] output.
    ///
    /// Never produces an empty path: stripping requires `len >= 2`,
    /// wrapping only ever prepends.
    #[must_use]
    pub fn apply_to_files(self, mut files: Vec<FileInfo>) -> Vec<FileInfo> {
        match self {
            Self::Original => files,
            Self::Subfolder => {
                for f in &mut files {
                    if f.path.len() == 1 {
                        let name = f.path[0].clone();
                        f.path.insert(0, name);
                    }
                }
                files
            }
            Self::NoSubfolder => {
                // Per-entry rule, NOT a list-length gate: a multi-file torrent
                // containing exactly one file still has `[name, component]`
                // paths and must strip. Single-file = `[name]` (len 1) is
                // untouched by the `>= 2` guard.
                for f in &mut files {
                    if f.path.len() >= 2 {
                        f.path.remove(0);
                    }
                }
                files
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal torrent bencoded dict with extra keys sorted correctly.
    ///
    /// `before_info` contains keys that sort before "info" (e.g., "httpseeds").
    /// `after_info` contains keys that sort after "info" (e.g., "url-list").
    fn make_torrent_bytes_sorted(before_info: &[u8], after_info: &[u8]) -> Vec<u8> {
        // Minimal info dict: name, piece length, pieces (20 zero bytes), length
        let info = b"d6:lengthi1048576e4:name4:test12:piece lengthi262144e6:pieces20:\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00e";
        let mut buf = Vec::new();
        buf.push(b'd');
        buf.extend_from_slice(before_info);
        buf.extend_from_slice(b"4:info");
        buf.extend_from_slice(info);
        buf.extend_from_slice(after_info);
        buf.push(b'e');
        buf
    }

    #[test]
    fn url_list_single_string() {
        // url-list sorts after info
        let data = make_torrent_bytes_sorted(b"", b"8:url-list24:http://example.com/files");
        let meta = torrent_from_bytes(&data).unwrap();
        assert_eq!(meta.url_list, vec!["http://example.com/files"]);
    }

    #[test]
    fn url_list_multiple() {
        let data = make_torrent_bytes_sorted(
            b"",
            b"8:url-listl24:http://example.com/files26:http://mirror.example.com/e",
        );
        let meta = torrent_from_bytes(&data).unwrap();
        assert_eq!(meta.url_list.len(), 2);
        assert_eq!(meta.url_list[0], "http://example.com/files");
        assert_eq!(meta.url_list[1], "http://mirror.example.com/");
    }

    /// M254: `to_torrent_bytes` round-trips through the parser with an
    /// identical info-hash (raw `info_bytes` path) and carries announce +
    /// url-list.
    #[test]
    fn m254_to_torrent_bytes_round_trips_info_hash() {
        let data = make_torrent_bytes_sorted(
            b"8:announce18:http://tr.example/",
            b"8:url-list24:http://example.com/files",
        );
        let meta = torrent_from_bytes(&data).unwrap();
        let out = meta.to_torrent_bytes().unwrap();
        let back = torrent_from_bytes(&out).unwrap();
        assert_eq!(meta.info_hash, back.info_hash, "info-hash must be exact");
        assert_eq!(back.announce.as_deref(), Some("http://tr.example/"));
        assert_eq!(back.url_list, vec!["http://example.com/files"]);
    }

    /// M254: the `InfoDict` serde fallback (`info_bytes` = None) produces
    /// parseable output with the same info-hash for a canonical source.
    #[test]
    fn m254_to_torrent_bytes_fallback_without_raw_info() {
        let data = make_torrent_bytes_sorted(b"8:announce18:http://tr.example/", b"");
        let mut meta = torrent_from_bytes(&data).unwrap();
        meta.info_bytes = None;
        let out = meta.to_torrent_bytes().unwrap();
        let back = torrent_from_bytes(&out).unwrap();
        // The fixture is canonical (sorted keys), so the serde
        // re-serialization reproduces the same info-hash.
        assert_eq!(meta.info_hash, back.info_hash);
        assert_eq!(back.info.name, meta.info.name);
    }

    #[test]
    fn url_list_absent() {
        let data = make_torrent_bytes_sorted(b"", b"");
        let meta = torrent_from_bytes(&data).unwrap();
        assert!(meta.url_list.is_empty());
    }

    #[test]
    fn httpseeds_present() {
        // httpseeds sorts before info
        let data = make_torrent_bytes_sorted(b"9:httpseedsl28:http://seed.example.com/seede", b"");
        let meta = torrent_from_bytes(&data).unwrap();
        assert_eq!(meta.httpseeds, vec!["http://seed.example.com/seed"]);
    }

    #[test]
    fn httpseeds_absent() {
        let data = make_torrent_bytes_sorted(b"", b"");
        let meta = torrent_from_bytes(&data).unwrap();
        assert!(meta.httpseeds.is_empty());
    }

    #[test]
    fn torrent_from_bytes_stores_raw_info_bytes() {
        let data = make_torrent_bytes_sorted(b"", b"");
        let meta = torrent_from_bytes(&data).unwrap();
        assert!(meta.info_bytes.is_some());
        let info_bytes = meta.info_bytes.unwrap();
        // Re-hashing the stored bytes should produce the same info hash
        let rehash = crate::sha1(&info_bytes);
        assert_eq!(rehash, meta.info_hash);
    }

    #[test]
    fn ssl_cert_parsed_from_info_dict() {
        // Build a torrent with ssl-cert in the info dict.
        let cert_pem = b"-----BEGIN CERTIFICATE-----\nMIIBtest\n-----END CERTIFICATE-----\n";
        let cert_len = cert_pem.len();

        // Minimal info dict with ssl-cert inserted (keys must be sorted)
        let mut info = Vec::new();
        info.extend_from_slice(b"d");
        info.extend_from_slice(b"6:lengthi1048576e");
        info.extend_from_slice(b"4:name4:test");
        info.extend_from_slice(b"12:piece lengthi262144e");
        info.extend_from_slice(b"6:pieces20:");
        info.extend_from_slice(&[0u8; 20]);
        info.extend_from_slice(format!("8:ssl-cert{cert_len}:").as_bytes());
        info.extend_from_slice(cert_pem);
        info.extend_from_slice(b"e");

        let mut torrent = Vec::new();
        torrent.extend_from_slice(b"d4:info");
        torrent.extend_from_slice(&info);
        torrent.extend_from_slice(b"e");

        let meta = torrent_from_bytes(&torrent).unwrap();
        assert!(meta.ssl_cert.is_some());
        assert_eq!(meta.ssl_cert.as_deref().unwrap(), cert_pem);
        assert_eq!(meta.info.ssl_cert.as_deref().unwrap(), cert_pem);
    }

    #[test]
    fn ssl_cert_absent_by_default() {
        let data = make_torrent_bytes_sorted(b"", b"");
        let meta = torrent_from_bytes(&data).unwrap();
        assert!(meta.ssl_cert.is_none());
        assert!(meta.info.ssl_cert.is_none());
    }

    /// Build a minimal info dict with optional `similar` and `collections` entries,
    /// wrapped in an outer torrent dict.  Keys are kept in bencode-sorted order.
    fn make_torrent_with_bep38(similar: Option<&[u8]>, collections: Option<&[u8]>) -> Vec<u8> {
        let mut info = Vec::new();
        info.extend_from_slice(b"d");
        // "collections" < "length" — insert first if present.
        if let Some(c) = collections {
            info.extend_from_slice(b"11:collections");
            info.extend_from_slice(c);
        }
        info.extend_from_slice(b"6:lengthi1048576e");
        info.extend_from_slice(b"4:name4:test");
        info.extend_from_slice(b"12:piece lengthi262144e");
        info.extend_from_slice(b"6:pieces20:");
        info.extend_from_slice(&[0u8; 20]);
        // "similar" > "pieces" and < "source"/"ssl-cert"
        if let Some(s) = similar {
            info.extend_from_slice(b"7:similar");
            info.extend_from_slice(s);
        }
        info.extend_from_slice(b"e");

        let mut torrent = Vec::new();
        torrent.extend_from_slice(b"d4:info");
        torrent.extend_from_slice(&info);
        torrent.extend_from_slice(b"e");
        torrent
    }

    #[test]
    fn parse_similar_torrents_from_info() {
        let hash_a = [0xAAu8; 20];
        let hash_b = [0xBBu8; 20];

        // Build bencode list: l20:<hash_a>20:<hash_b>e
        let mut similar_list = Vec::new();
        similar_list.extend_from_slice(b"l");
        similar_list.extend_from_slice(b"20:");
        similar_list.extend_from_slice(&hash_a);
        similar_list.extend_from_slice(b"20:");
        similar_list.extend_from_slice(&hash_b);
        similar_list.extend_from_slice(b"e");

        let data = make_torrent_with_bep38(Some(&similar_list), None);
        let meta = torrent_from_bytes(&data).expect("parse should succeed");

        assert_eq!(meta.info.similar.len(), 2);
        assert_eq!(meta.info.similar[0], Id20(hash_a));
        assert_eq!(meta.info.similar[1], Id20(hash_b));
    }

    #[test]
    fn parse_collections_from_info() {
        // Build bencode list: l6:movies6:sci-fie
        let collections_list = b"l6:movies6:sci-fie";

        let data = make_torrent_with_bep38(None, Some(collections_list));
        let meta = torrent_from_bytes(&data).expect("parse should succeed");

        assert_eq!(meta.info.collections.len(), 2);
        assert_eq!(meta.info.collections[0], "movies");
        assert_eq!(meta.info.collections[1], "sci-fi");
    }

    #[test]
    fn similar_empty_when_absent() {
        let data = make_torrent_bytes_sorted(b"", b"");
        let meta = torrent_from_bytes(&data).expect("parse should succeed");
        assert!(meta.info.similar.is_empty());
        assert!(meta.info.collections.is_empty());
    }

    #[test]
    fn similar_ignores_wrong_length_hashes() {
        let valid_hash = [0xCCu8; 20];
        let too_short = [0xDDu8; 19];
        let too_long = [0xEEu8; 21];

        // Build bencode list with mixed entries: 19-byte, 20-byte valid, 21-byte
        let mut similar_list = Vec::new();
        similar_list.extend_from_slice(b"l");
        // 19 bytes — invalid
        similar_list.extend_from_slice(b"19:");
        similar_list.extend_from_slice(&too_short);
        // 20 bytes — valid
        similar_list.extend_from_slice(b"20:");
        similar_list.extend_from_slice(&valid_hash);
        // 21 bytes — invalid
        similar_list.extend_from_slice(b"21:");
        similar_list.extend_from_slice(&too_long);
        similar_list.extend_from_slice(b"e");

        let data = make_torrent_with_bep38(Some(&similar_list), None);
        let meta = torrent_from_bytes(&data).expect("parse should succeed");

        // Only the 20-byte entry survives.
        assert_eq!(meta.info.similar.len(), 1);
        assert_eq!(meta.info.similar[0], Id20(valid_hash));
    }

    #[test]
    fn m252_content_layout_original_is_identity() {
        let files = vec![
            FileInfo {
                path: vec!["root".into(), "a.bin".into()],
                length: 1,
            },
            FileInfo {
                path: vec!["root".into(), "sub".into(), "b.bin".into()],
                length: 2,
            },
        ];
        let out = ContentLayout::Original.apply_to_files(files.clone());
        assert_eq!(out, files);
    }

    #[test]
    fn m252_content_layout_subfolder_wraps_single_file_only() {
        let single = vec![FileInfo {
            path: vec!["movie.mkv".into()],
            length: 9,
        }];
        let out = ContentLayout::Subfolder.apply_to_files(single);
        assert_eq!(
            out[0].path,
            vec!["movie.mkv".to_string(), "movie.mkv".to_string()]
        );

        let multi = vec![FileInfo {
            path: vec!["root".into(), "a.bin".into()],
            length: 1,
        }];
        let out = ContentLayout::Subfolder.apply_to_files(multi.clone());
        assert_eq!(
            out, multi,
            "multi-file already has a root — Subfolder is a no-op"
        );
    }

    #[test]
    fn m252_content_layout_nosubfolder_strips_root_per_entry() {
        let multi = vec![
            FileInfo {
                path: vec!["root".into(), "a.bin".into()],
                length: 1,
            },
            FileInfo {
                path: vec!["root".into(), "sub".into(), "b.bin".into()],
                length: 2,
            },
        ];
        let out = ContentLayout::NoSubfolder.apply_to_files(multi);
        assert_eq!(out[0].path, vec!["a.bin".to_string()]);
        assert_eq!(out[1].path, vec!["sub".to_string(), "b.bin".to_string()]);

        let single = vec![FileInfo {
            path: vec!["movie.mkv".into()],
            length: 9,
        }];
        let out = ContentLayout::NoSubfolder.apply_to_files(single.clone());
        assert_eq!(
            out, single,
            "single-file is already flat — NoSubfolder is a no-op"
        );

        // OV F1: a multi-file torrent with exactly ONE file entry still
        // carries the root component and must strip.
        let one_entry_multi = vec![FileInfo {
            path: vec!["root".into(), "only.bin".into()],
            length: 3,
        }];
        let out = ContentLayout::NoSubfolder.apply_to_files(one_entry_multi);
        assert_eq!(out[0].path, vec!["only.bin".to_string()]);
    }

    #[test]
    fn m252_content_layout_serde_snake_case_round_trip() {
        let j = serde_json::to_string(&ContentLayout::NoSubfolder).unwrap();
        assert_eq!(j, "\"no_subfolder\"");
        let back: ContentLayout = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ContentLayout::NoSubfolder);
        assert_eq!(ContentLayout::default(), ContentLayout::Original);
    }
}
