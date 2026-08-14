#![forbid(unsafe_code)]
//! Core `BitTorrent` types used by the crawler workspace: info-hash types,
//! network address families, and SHA-1/SHA-256 digests.
//!
//! This is a trimmed subset of the upstream irontide `gaia-core` crate —
//! only the surface consumed by `gaia-dht`, `gaia-wire`, and the crawler.

mod crc32c;
mod error;
mod hash;

pub use crc32c::crc32c;
pub use error::{Error, Result};
pub use hash::{Id20, Id32};

/// Network address family for dual-stack support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressFamily {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

/// Compute SHA1 hash of input bytes.
#[must_use]
pub fn sha1(data: &[u8]) -> Id20 {
    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, data);
    let mut id = [0u8; 20];
    id.copy_from_slice(hash.as_ref());
    Id20(id)
}

/// Compute SHA-256 hash of input bytes (used by `BitTorrent` v2, BEP 52).
#[must_use]
pub fn sha256(data: &[u8]) -> Id32 {
    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data);
    let mut id = [0u8; 32];
    id.copy_from_slice(hash.as_ref());
    Id32(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_string() {
        let hash = sha256(b"");
        assert_eq!(
            hash.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        let hash = sha256(b"hello");
        assert_eq!(
            hash.to_hex(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
