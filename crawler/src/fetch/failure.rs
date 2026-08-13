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
            Self::Other
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
            FetchFailureKind::Other,
        ] {
            assert!(seen.insert(k.as_str().to_string()), "dup: {}", k.as_str());
        }
    }
}
