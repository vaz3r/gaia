#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "M175: peer ID — fixed 20-byte BEP 3 client identifier"
)]

use crate::hash::Id20;

/// A `BitTorrent` peer ID (20 bytes).
///
/// Uses Azureus-style encoding: `-FE0100-` followed by 12 random bytes.
/// FE = Torrent (formerly Ferrite), 0100 = version 0.1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub Id20);

impl PeerId {
    /// Client identifier prefix.
    const PREFIX: &'static [u8] = b"-FE0100-";

    /// Generate a new random peer ID with the default Torrent prefix.
    #[must_use]
    pub fn generate() -> Self {
        Self::generate_with_prefix(Self::PREFIX)
    }

    /// Generate an anonymous peer ID with a generic client prefix.
    ///
    /// Uses `-XX0000-` prefix (generic/unknown client) instead of `-FE0100-`
    /// to avoid identifying the client software.
    #[must_use]
    pub fn generate_anonymous() -> Self {
        Self::generate_with_prefix(b"-XX0000-")
    }

    /// Generate a peer ID with the given 8-byte Azureus-style prefix.
    fn generate_with_prefix(prefix: &[u8]) -> Self {
        let mut bytes = [0u8; 20];
        bytes[..8].copy_from_slice(prefix);
        for byte in &mut bytes[8..] {
            *byte = random_byte();
        }
        Self(Id20(bytes))
    }

    /// Return the raw 20 bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 20] {
        self.0.as_bytes()
    }

    /// Return the client prefix (e.g., "-FE0100-").
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.0.0[..8]
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show prefix as ASCII, rest as hex
        let prefix = std::str::from_utf8(&self.0.0[..8]).unwrap_or("????????");
        let suffix = hex::encode(&self.0.0[8..]);
        write!(f, "{prefix}{suffix}")
    }
}

/// Simple random byte using thread-local state seeded from system time.
///
/// Backed by the shared [`crate::xorshift64_step`] helper to keep peer
/// ID generation in lockstep with other in-tree consumers (sim
/// per-link RNG state).
pub(crate) fn random_byte() -> u8 {
    use std::cell::Cell;
    use std::time::SystemTime;

    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        );
    }

    STATE.with(|s| {
        let next = crate::xorshift64_step(s.get().max(1));
        s.set(next);
        next as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_has_prefix() {
        let id = PeerId::generate();
        assert_eq!(id.prefix(), b"-FE0100-");
    }

    #[test]
    fn peer_ids_are_unique() {
        let a = PeerId::generate();
        let b = PeerId::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn anonymous_peer_id_has_generic_prefix() {
        let id = PeerId::generate_anonymous();
        assert_eq!(id.prefix(), b"-XX0000-");
    }

    #[test]
    fn anonymous_peer_ids_are_unique() {
        let a = PeerId::generate_anonymous();
        let b = PeerId::generate_anonymous();
        assert_ne!(a, b);
    }

    #[test]
    fn peer_id_display() {
        let id = PeerId::generate();
        let s = format!("{id}");
        assert!(s.starts_with("-FE0100-"));
        assert_eq!(s.len(), 8 + 24); // 8 ASCII prefix + 12 bytes as hex
    }

    /// Regression guard for the [`random_byte`] refactor onto
    /// [`crate::xorshift64_step`]. Fills a 1024-byte buffer with output
    /// and asserts (a) at least one byte is non-zero and (b) the
    /// stddev sits in the band a uniform distribution would produce.
    /// Catches the failure mode where a re-implementation accidentally
    /// loses entropy (e.g. by misordering the shifts).
    #[test]
    fn random_byte_regression_uniform_distribution() {
        let mut buf = [0u8; 1024];
        for slot in &mut buf {
            *slot = random_byte();
        }
        let nonzero = buf.iter().filter(|&&b| b != 0).count();
        assert!(
            nonzero >= 1000,
            "≤4 non-zero bytes in 1024 samples is suspicious — got {nonzero} non-zero"
        );
        // Empirical stddev for u8 uniform-ish is ~73.9; a uniform sample
        // typically produces 60..80 over a 1024-sample window.
        let mean: f64 = buf.iter().map(|&b| f64::from(b)).sum::<f64>() / 1024.0;
        let var: f64 = buf
            .iter()
            .map(|&b| {
                let d = f64::from(b) - mean;
                d * d
            })
            .sum::<f64>()
            / 1024.0;
        let stddev = var.sqrt();
        assert!(
            (60.0..=80.0).contains(&stddev),
            "stddev {stddev:.2} fell outside the [60.0, 80.0] uniform-distribution band"
        );
    }
}
