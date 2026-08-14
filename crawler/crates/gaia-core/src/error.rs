/// Result type alias for irontide-core operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from core `BitTorrent` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid hex string.
    #[error("invalid hex: {0}")]
    InvalidHex(String),

    /// Invalid hash length.
    #[error("invalid hash length: expected {expected}, got {got}")]
    InvalidHashLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length received.
        got: usize,
    },
}
