//! Typed classification of peer-fetch failures.
//!
//! The previous classifiers (`classify_error` / `classify_peer_error`) sniffed
//! the formatted `anyhow` error string with `msg.contains(...)` and dumped
//! everything unrecognized into `other` — which is why ~11% of fetch attempts
//! (≈34k/15min into `peer_errors_other`) were unexplained. This enum is the
//! single source of truth for the taxonomy, and `FetchError` carries the kind
//! from where the error is actually produced.

/// Granular cause of one peer-fetch failure. Persisted as its string form in
/// SQLite `scanned.failure_reason` (via `as_str()`); the dashboard and the
/// peer-failure-breakdown log line aggregate these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchFailureKind {
    /// Connect or handshake exchange exceeded a per-peer timeout.
    Timeout,
    /// TCP connect refused (peer offline / not listening).
    ConnectRefused,
    /// Connection reset by the peer mid-exchange.
    ConnectionReset,
    /// Peer closed the connection (EOF) mid-exchange.
    ConnectionClosed,
    /// Peer did not complete a valid BEP 10 extension handshake.
    HandshakeFailed,
    /// Peer handshake completed but ut_metadata is not advertised.
    NoUtMetadata,
    /// Peer rejected a ut_metadata piece request.
    MetadataRejected,
    /// Bencode / metadata decode or length validation failed.
    ParseError,
    /// Assembled info SHA-1 did not match the requested infohash.
    Sha1Mismatch,
    /// Gave up after `EARLY_ABORT_DIALS` consecutive dead dials.
    EarlyAbort,
    /// Overall per-hash `FETCH_DEADLINE` expired.
    Deadline,
    /// get_peers returned no dialable peers at all.
    EmptyPeers,
    /// A DHT `get_peers` lookup failed during the fetch (infrastructure
    /// failure — transient, retry-productive).
    DhtLookupFailed,
    /// The fetch could not acquire a lookup-pool permit (infrastructure
    /// failure — transient, retry-productive).
    LookupPoolExhausted,
    /// Anything not covered above (kept so no failure is dropped).
    Other,
}

impl FetchFailureKind {
    /// Stable string form persisted to SQLite / surfaced in logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::ConnectRefused => "connect_refused",
            Self::ConnectionReset => "connection_reset",
            Self::ConnectionClosed => "connection_closed",
            Self::HandshakeFailed => "handshake_failed",
            Self::NoUtMetadata => "no_ut_metadata",
            Self::MetadataRejected => "metadata_rejected",
            Self::ParseError => "parse_error",
            Self::Sha1Mismatch => "sha1_mismatch",
            Self::EarlyAbort => "early_abort",
            Self::Deadline => "deadline",
            Self::EmptyPeers => "empty_peers",
            Self::DhtLookupFailed => "dht_lookup_failed",
            Self::LookupPoolExhausted => "lookup_pool_exhausted",
            Self::Other => "other",
        }
    }

    /// Classify a peer-fetch error by its underlying cause. Prefer passing the
    /// raw `std::io::Error` when available; the fallback string path catches
    /// contextualized `anyhow` errors and keeps a stable mapping to the
    /// variants that `fetch_from_peer` actually produces.
    pub fn from_error(e: &anyhow::Error) -> Self {
        // io::Error carries a precise ErrorKind — the most reliable signal.
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            if let Some(kind) = Self::from_io_kind(&io.kind()) {
                return kind;
            }
        }
        Self::from_string(&e.to_string())
    }

    /// Map an `std::io::ErrorKind` to a failure kind, or `None` for kinds we
    /// don't special-case (caller falls back to string sniffing).
    pub fn from_io_kind(kind: &std::io::ErrorKind) -> Option<Self> {
        use std::io::ErrorKind;
        Some(match kind {
            ErrorKind::TimedOut => Self::Timeout,
            ErrorKind::ConnectionRefused => Self::ConnectRefused,
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => Self::ConnectionReset,
            ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe => Self::ConnectionClosed,
            ErrorKind::InvalidData => Self::ParseError,
            _ => return None,
        })
    }

    /// Classify from the formatted error message (fallback when no `io::Error`
    /// is available). Mirrors the message shapes `fetch_from_peer` produces.
    fn from_string(msg: &str) -> Self {
        if msg.contains("timed out") || msg.contains("timeout") {
            Self::Timeout
        } else if msg.contains("Connection refused") || msg.contains("connection refused") {
            Self::ConnectRefused
        } else if msg.contains("connection reset") || msg.contains("reset by peer") {
            Self::ConnectionReset
        } else if msg.contains("connection closed") {
            Self::ConnectionClosed
        } else if msg.contains("does not support BEP 10") {
            Self::HandshakeFailed
        } else if msg.contains("does not advertise ut_metadata") {
            Self::NoUtMetadata
        } else if msg.contains("rejected metadata piece") {
            Self::MetadataRejected
        } else if msg.contains("SHA-1 mismatch") || msg.contains("size mismatch") {
            Self::Sha1Mismatch
        } else if msg.contains("invalid message")
            || msg.contains("invalid bencode")
            || msg.contains("invalid handshake")
            || msg.contains("metadata size")
            || msg.contains("zero metadata")
        {
            Self::ParseError
        } else {
            // Unmatched fallback: log so taxonomy gaps stay visible instead of
            // silently folding into `other`.
            tracing::debug!(unmatched_failure = msg, "unmatched failure classification");
            Self::Other
        }
    }

    /// Parse a failure kind from its stable string form (as returned by
    /// `as_str()`). Used to reconstruct a kind from the persisted
    /// `failure_reason` or `dominant_failure` string.
    pub fn from_str(s: &str) -> Self {
        match s {
            "timeout" => Self::Timeout,
            "connect_refused" => Self::ConnectRefused,
            "connection_reset" => Self::ConnectionReset,
            "connection_closed" => Self::ConnectionClosed,
            "handshake_failed" => Self::HandshakeFailed,
            "no_ut_metadata" => Self::NoUtMetadata,
            "metadata_rejected" => Self::MetadataRejected,
            "parse_error" => Self::ParseError,
            "sha1_mismatch" => Self::Sha1Mismatch,
            "early_abort" => Self::EarlyAbort,
            "deadline" => Self::Deadline,
            "empty_peers" => Self::EmptyPeers,
            "dht_lookup_failed" => Self::DhtLookupFailed,
            "lookup_pool_exhausted" => Self::LookupPoolExhausted,
            _ => Self::Other,
        }
    }
}

/// Maximum attempts per failure class. Transient classes (network/backend
/// failures that often recover) get a generous budget; dead-verdict classes
/// (peer said no / hash is dead) are capped low so dead-hash churn stays
/// bounded. `kind` is the persisted `failure_reason` string (`None` = the
/// legacy "unknown" bucket, treated as transient).
pub fn retry_cap(kind: Option<&str>) -> u32 {
    match kind {
        Some("empty_peers")
        | Some("no_ut_metadata")
        | Some("metadata_rejected")
        | Some("sha1_mismatch")
        | Some("parse_error") => 2,
        _ => 4,
    }
}

/// Backoff before the next attempt, per class. Transient classes use a shorter
/// schedule (so a recoverable hash is retried promptly); dead-verdict classes
/// use the longer exponential backoff and are capped at 2 attempts anyway.
pub fn retry_delay(kind: Option<&str>, attempts: i64) -> i64 {
    match kind {
        Some("empty_peers")
        | Some("no_ut_metadata")
        | Some("metadata_rejected")
        | Some("sha1_mismatch")
        | Some("parse_error") => crate::storage::backoff_secs(attempts),
        _ => {
            // Transient: 1m, 2m, 4m, 8m, ... capped at 10m.
            let n = attempts.max(1) - 1;
            let shift = n.min(10);
            (60i64.saturating_mul(1i64 << shift)).min(600)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn io_kind_mapping() {
        assert_eq!(
            FetchFailureKind::from_io_kind(&std::io::ErrorKind::TimedOut),
            Some(FetchFailureKind::Timeout)
        );
        assert_eq!(
            FetchFailureKind::from_io_kind(&std::io::ErrorKind::ConnectionRefused),
            Some(FetchFailureKind::ConnectRefused)
        );
        assert_eq!(
            FetchFailureKind::from_io_kind(&std::io::ErrorKind::ConnectionReset),
            Some(FetchFailureKind::ConnectionReset)
        );
        assert_eq!(
            FetchFailureKind::from_io_kind(&std::io::ErrorKind::UnexpectedEof),
            Some(FetchFailureKind::ConnectionClosed)
        );
        assert_eq!(
            FetchFailureKind::from_io_kind(&std::io::ErrorKind::InvalidData),
            Some(FetchFailureKind::ParseError)
        );
        assert_eq!(FetchFailureKind::from_io_kind(&std::io::ErrorKind::Other), None);
    }

    #[test]
    fn io_error_downcasts_to_kind() {
        let io: std::io::Error = std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        );
        let e = anyhow!(io);
        assert_eq!(
            FetchFailureKind::from_error(&e),
            FetchFailureKind::ConnectionReset
        );
    }

    #[test]
    fn string_forms_map_to_expected_kinds() {
        let cases: &[(&str, FetchFailureKind)] = &[
            ("connect to 1.2.3.4:6881 timed out", FetchFailureKind::Timeout),
            ("timed out waiting for peer handshake", FetchFailureKind::Timeout),
            ("Connection refused", FetchFailureKind::ConnectRefused),
            ("connection reset by peer", FetchFailureKind::ConnectionReset),
            ("connection closed during handshake", FetchFailureKind::ConnectionClosed),
            ("connection closed while fetching metadata", FetchFailureKind::ConnectionClosed),
            ("peer 1.2.3.4:6881 does not support BEP 10 extensions", FetchFailureKind::HandshakeFailed),
            ("peer 1.2.3.4:6881 does not advertise ut_metadata", FetchFailureKind::NoUtMetadata),
            ("peer 1.2.3.4:6881 rejected metadata piece 3", FetchFailureKind::MetadataRejected),
            ("invalid message from peer", FetchFailureKind::ParseError),
            ("peer advertised out-of-range metadata size", FetchFailureKind::ParseError),
            ("peer 1.2.3.4:6881 advertised zero metadata size", FetchFailureKind::ParseError),
            ("SHA-1 mismatch", FetchFailureKind::Sha1Mismatch),
            ("assembled metadata size mismatch", FetchFailureKind::Sha1Mismatch),
            ("something unexpected happened", FetchFailureKind::Other),
        ];
        for (msg, expected) in cases {
            let e = anyhow!("{msg}");
            assert_eq!(
                FetchFailureKind::from_error(&e),
                *expected,
                "message: {msg}"
            );
        }
    }

    #[test]
    fn baits_do_not_swallow_a_real_error() {
        // A connect timeout must not be re-classified by a later substring.
        let e = anyhow!("connect to 1.2.3.4:6881 timed out");
        assert_eq!(FetchFailureKind::from_error(&e), FetchFailureKind::Timeout);
    }

    #[test]
    fn as_str_is_stable_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for k in [
            FetchFailureKind::Timeout,
            FetchFailureKind::ConnectRefused,
            FetchFailureKind::ConnectionReset,
            FetchFailureKind::ConnectionClosed,
            FetchFailureKind::HandshakeFailed,
            FetchFailureKind::NoUtMetadata,
            FetchFailureKind::MetadataRejected,
            FetchFailureKind::ParseError,
            FetchFailureKind::Sha1Mismatch,
            FetchFailureKind::EarlyAbort,
            FetchFailureKind::Deadline,
            FetchFailureKind::EmptyPeers,
            FetchFailureKind::DhtLookupFailed,
            FetchFailureKind::LookupPoolExhausted,
            FetchFailureKind::Other,
        ] {
            assert!(seen.insert(k.as_str().to_string()), "dup: {}", k.as_str());
        }
    }

    #[test]
    fn retry_cap_varies_by_class() {
        assert_eq!(retry_cap(Some("empty_peers")), 2);
        assert_eq!(retry_cap(Some("no_ut_metadata")), 2);
        assert_eq!(retry_cap(Some("metadata_rejected")), 2);
        assert_eq!(retry_cap(Some("sha1_mismatch")), 2);
        assert_eq!(retry_cap(Some("parse_error")), 2);
        assert_eq!(retry_cap(Some("timeout")), 4);
        assert_eq!(retry_cap(Some("deadline")), 4);
        assert_eq!(retry_cap(Some("connect_refused")), 4);
        assert_eq!(retry_cap(Some("dht_lookup_failed")), 4);
        assert_eq!(retry_cap(Some("lookup_pool_exhausted")), 4);
        assert_eq!(retry_cap(None), 4);
        assert_eq!(retry_cap(Some("unknown_reason")), 4);
    }

    #[test]
    fn retry_delay_varies_by_class() {
        // Dead-verdict: long exponential (backoff_secs).
        assert_eq!(retry_delay(Some("empty_peers"), 1), 60);
        assert_eq!(retry_delay(Some("empty_peers"), 2), 120);
        // Transient: short, capped at 600s.
        assert_eq!(retry_delay(Some("timeout"), 1), 60);
        assert_eq!(retry_delay(Some("timeout"), 2), 120);
        assert_eq!(retry_delay(Some("timeout"), 3), 240);
        assert_eq!(retry_delay(Some("timeout"), 4), 480);
        assert_eq!(retry_delay(Some("timeout"), 10), 600);
        assert_eq!(retry_delay(None, 1), 60);
    }
}
