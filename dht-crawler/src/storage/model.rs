/// Exponential backoff in seconds: 5m, 10m, 20m, ... capped at 6h.
pub fn backoff_secs(attempts: i64) -> i64 {
    const BASE: i64 = 300;
    const MAX: i64 = 6 * 3600;
    let n = attempts.max(1) - 1;
    let shift = n.min(30);
    let secs = BASE.saturating_mul(1i64 << shift);
    secs.min(MAX)
}

/// A single accepted torrent record, keyed by its 20-byte info hash. Holds
/// torrent metadata only; classification lives in a future `torrent_details`
/// table and is re-derivable from `scanned.info_bytes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentRecord {
    pub info_hash: [u8; 20],
    pub name: String,
    pub size_bytes: Option<i64>,
    pub file_count: Option<i64>,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// Outcome of a metadata fetch attempt for an infohash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannedStatus {
    /// Metadata fetched, SHA-1 verified, and accepted.
    Ok,
    /// Metadata fetched and verified but filtered out (not movie/TV).
    Skipped,
    /// Metadata could not be fetched; `attempts` and `next_attempt` drive
    /// exponential backoff. `failure_reason` is the dominant failure class.
    Failed {
        attempts: i64,
        next_attempt: i64,
        failure_reason: Option<String>,
    },
}

/// A row in the `scanned` table. `info_bytes` holds the raw bencoded `info`
/// dictionary so classification / enrichment can be re-run offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedRecord {
    pub info_hash: [u8; 20],
    pub status: ScannedStatus,
    pub info_bytes: Option<Vec<u8>>,
    pub raw_name: Option<String>,
    pub last_attempt: i64,
}
