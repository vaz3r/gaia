//! BEP 33: 256-byte bloom filter for DHT scrape swarm estimation.
//!
//! When a DHT node responds to a `get_peers` query with `scrape=1`, it may
//! include `BFpe` (peer/leecher bloom filter) and `BFsd` (seed bloom filter)
//! in the response. Each filter is a 2048-bit (256-byte) bloom filter with
//! k=2 hash functions derived from SHA-1 of the peer's compact address.

use std::net::SocketAddr;

use gaia_core::sha1;

/// BEP 33: 256-byte (2048-bit) bloom filter for DHT scrape swarm estimation.
///
/// Uses k=2 hash functions derived from SHA-1 of the peer's IP+port bytes.
/// The two 11-bit indices are extracted from the first 4 bytes of the hash.
#[derive(Debug, Clone)]
pub struct ScrapeBloomFilter {
    bits: [u8; 256],
}

impl ScrapeBloomFilter {
    /// Create an empty bloom filter with all bits cleared.
    #[must_use]
    pub fn new() -> Self {
        Self { bits: [0u8; 256] }
    }

    /// Insert a peer address into the filter.
    ///
    /// Per BEP 33: compute SHA-1 of the compact IP+port bytes, then use the
    /// first 4 bytes as two 11-bit indices (k=2, m=2048).
    pub fn insert(&mut self, addr: SocketAddr) {
        let hash = sha1_of_addr(addr);
        let (index1, index2) = indices_from_hash(&hash);
        self.bits[index1 / 8] |= 1 << (index1 % 8);
        self.bits[index2 / 8] |= 1 << (index2 % 8);
    }

    /// Estimate the number of items inserted into the filter from bit density.
    ///
    /// BEP 33 formula: `count = ln(1 - bits_set/m) * m / -k` where m=2048, k=2.
    /// Returns 0 for an empty filter and `u32::MAX` for a saturated filter.
    #[must_use]
    pub fn estimate_count(&self) -> u32 {
        let bits_set: u32 = self.bits.iter().map(|b| b.count_ones()).sum();
        if bits_set == 0 {
            return 0;
        }
        if bits_set >= 2048 {
            return u32::MAX;
        }
        let m = 2048.0_f64;
        let k = 2.0_f64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let count = (-(m / k) * (1.0 - f64::from(bits_set) / m).ln()) as u32;
        count
    }

    /// Return the raw 256-byte filter data.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 256] {
        &self.bits
    }

    /// Construct a bloom filter from raw bytes. Returns `None` if the slice
    /// is not exactly 256 bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 256 {
            return None;
        }
        let mut bits = [0u8; 256];
        bits.copy_from_slice(bytes);
        Some(Self { bits })
    }
}

impl Default for ScrapeBloomFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute SHA-1 of the compact peer address bytes (IP octets + port in
/// network byte order).
fn sha1_of_addr(addr: SocketAddr) -> [u8; 20] {
    let compact = match addr {
        SocketAddr::V4(v4) => {
            let mut buf = [0u8; 6];
            buf[..4].copy_from_slice(&v4.ip().octets());
            buf[4..6].copy_from_slice(&v4.port().to_be_bytes());
            buf.to_vec()
        }
        SocketAddr::V6(v6) => {
            let mut buf = [0u8; 18];
            buf[..16].copy_from_slice(&v6.ip().octets());
            buf[16..18].copy_from_slice(&v6.port().to_be_bytes());
            buf.to_vec()
        }
    };
    sha1(&compact).0
}

/// Extract two 11-bit indices from the first 4 bytes of a SHA-1 hash.
fn indices_from_hash(hash: &[u8; 20]) -> (usize, usize) {
    let index1 = (u16::from_be_bytes([hash[0], hash[1]]) % 2048) as usize;
    let index2 = (u16::from_be_bytes([hash[2], hash[3]]) % 2048) as usize;
    (index1, index2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_insert_and_check_bits() {
        let mut filter = ScrapeBloomFilter::new();
        let addr: SocketAddr = "192.168.1.1:6881".parse().expect("valid addr");
        filter.insert(addr);

        // Exactly two bits should be set (or one if both indices collide).
        let bits_set: u32 = filter.bits.iter().map(|b| b.count_ones()).sum();
        assert!(
            bits_set == 1 || bits_set == 2,
            "expected 1 or 2 bits set, got {bits_set}"
        );
    }

    #[test]
    fn bloom_estimate_count_empty() {
        let filter = ScrapeBloomFilter::new();
        assert_eq!(filter.estimate_count(), 0);
    }

    #[test]
    fn bloom_estimate_count_small() {
        let mut filter = ScrapeBloomFilter::new();
        for i in 0..5u16 {
            let addr: SocketAddr = format!("10.0.0.{}:{}", i / 256, 6881 + i)
                .parse()
                .expect("valid addr");
            filter.insert(addr);
        }
        let estimate = filter.estimate_count();
        // With 5 inserts, estimate should be roughly 5 (within 50%).
        assert!(
            (3..=8).contains(&estimate),
            "expected estimate near 5, got {estimate}"
        );
    }

    #[test]
    fn bloom_estimate_count_medium() {
        let mut filter = ScrapeBloomFilter::new();
        for i in 0..50u16 {
            let addr: SocketAddr = format!("10.{}.{}.{}:6881", i / 256, (i / 16) % 256, i % 256)
                .parse()
                .expect("valid addr");
            filter.insert(addr);
        }
        let estimate = filter.estimate_count();
        // With 50 inserts, estimate should be within 30% of 50.
        assert!(
            (35..=65).contains(&estimate),
            "expected estimate near 50, got {estimate}"
        );
    }

    #[test]
    fn bloom_serialize_roundtrip() {
        let mut filter = ScrapeBloomFilter::new();
        let addr: SocketAddr = "10.0.0.1:6881".parse().expect("valid addr");
        filter.insert(addr);

        let bytes = filter.as_bytes();
        let restored = ScrapeBloomFilter::from_bytes(bytes).expect("valid 256 bytes");
        assert_eq!(filter.bits, restored.bits);
    }

    #[test]
    fn bloom_from_bytes_wrong_size() {
        assert!(ScrapeBloomFilter::from_bytes(&[0u8; 128]).is_none());
        assert!(ScrapeBloomFilter::from_bytes(&[0u8; 0]).is_none());
        assert!(ScrapeBloomFilter::from_bytes(&[0u8; 257]).is_none());
    }
}
