//! A small, dependency-free bloom filter used to short-circuit per-hash
//! database `scan_blocked_batch` reads on the sampler hot path.
//!
//! The bloom caches *known-blocked* verdicts: when a hash's authoritative DB
//! check says "skip this hash" (already accepted, filtered, or inside a
//! backoff window), the hash is inserted. Because a bloom filter has no false
//! negatives, a later *miss* proves the hash was never seen as blocked, so the
//! DB read can be skipped; a *hit* means the hash is known-blocked and is
//! skipped directly. The first time a hash is encountered always goes through
//! the authoritative DB check, keeping correctness despite a cold filter.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fixed-size bloom filter with `2^bits_power` bits and `k` hash functions.
#[derive(Debug, Clone)]
pub struct BloomFilter {
    bits: Vec<u64>,
    mask: u64,
    k: u32,
}

impl BloomFilter {
    /// Create a filter with `2^bits_power` bits and `k` hash functions.
    ///
    /// For ~10M entries at a ~1% false-positive rate, 2^27 bits (16 MiB) and
    /// k=7 is appropriate (m/n ≈ 13.4 bits per item).
    pub fn new(bits_power: u32, k: u32) -> Self {
        assert!((6..=48).contains(&bits_power), "bits_power out of range");
        assert!((1..=32).contains(&k), "k out of range");
        Self {
            bits: vec![0u64; 1 << (bits_power - 6)],
            mask: (1u64 << bits_power) - 1,
            k,
        }
    }

    /// Add `key` to the filter.
    pub fn insert(&mut self, key: &[u8]) {
        let (h1, h2) = hashes(key);
        for i in 0..self.k {
            let idx = index(h1, h2, i, self.mask);
            self.bits[(idx >> 6) as usize] |= 1u64 << (idx & 63);
        }
    }

    /// True if `key` may have been added.
    pub fn contains(&self, key: &[u8]) -> bool {
        let (h1, h2) = hashes(key);
        (0..self.k).all(|i| {
            let idx = index(h1, h2, i, self.mask);
            self.bits[(idx >> 6) as usize] & (1u64 << (idx & 63)) != 0
        })
    }

    /// Reset all bits to zero.
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }
}

/// Generational aging bloom filter. Maintains active (`current`) and previous (`prev`)
/// generations, rotating generations after `rotation_interval` (default 24h).
/// This prevents permanent poison-blacklisting of dead torrents (Bitmagnet stable bloom pattern).
#[derive(Debug)]
pub struct GenerationalBloom {
    current: BloomFilter,
    prev: BloomFilter,
    bits_power: u32,
    k: u32,
    last_rotation: Instant,
    rotation_interval: Duration,
}

impl GenerationalBloom {
    pub fn new(bits_power: u32, k: u32, rotation_interval: Duration) -> Self {
        Self {
            current: BloomFilter::new(bits_power, k),
            prev: BloomFilter::new(bits_power, k),
            bits_power,
            k,
            last_rotation: Instant::now(),
            rotation_interval,
        }
    }

    fn maybe_rotate(&mut self) {
        if self.last_rotation.elapsed() >= self.rotation_interval {
            self.prev = std::mem::replace(&mut self.current, BloomFilter::new(self.bits_power, self.k));
            self.last_rotation = Instant::now();
        }
    }

    pub fn insert(&mut self, key: &[u8]) {
        self.maybe_rotate();
        self.current.insert(key);
    }

    pub fn contains(&mut self, key: &[u8]) -> bool {
        self.maybe_rotate();
        self.current.contains(key) || self.prev.contains(key)
    }
}

/// Interior-mutable, cloneable generational bloom filter shared across sampler
/// loops and the fetcher without copying the backing bits.
#[derive(Debug, Clone)]
pub struct SharedBloom {
    inner: std::sync::Arc<Mutex<GenerationalBloom>>,
}

impl SharedBloom {
    pub fn new(bits_power: u32, k: u32) -> Self {
        Self::with_interval(bits_power, k, Duration::from_secs(24 * 3600))
    }

    pub fn with_interval(bits_power: u32, k: u32, rotation_interval: Duration) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(GenerationalBloom::new(bits_power, k, rotation_interval))),
        }
    }

    /// Add `key`.
    pub fn insert(&self, key: &[u8]) {
        self.inner.lock().unwrap().insert(key);
    }

    /// True if `key` may have been added in current or previous generation.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.inner.lock().unwrap().contains(key)
    }
}

/// Derive two 64-bit hashes from a 20-byte infohash (splitmix64 of the key
/// halves). Two independent hashes power the double-hashing scheme below.
fn hashes(key: &[u8]) -> (u64, u64) {
    let mut h1 = 0x9e37_79b9_7f4a_7c15u64;
    let mut h2 = 0xbf58_476d_1ce4_e5b9u64;
    for (i, b) in key.iter().enumerate() {
        if i % 2 == 0 {
            h1 ^= *b as u64;
            h1 = splitmix64(h1);
        } else {
            h2 ^= *b as u64;
            h2 = splitmix64(h2);
        }
    }
    (h1, h2)
}

/// Combined hashing: two independent 64-bit hashes produce `k` indices via
/// the classic double-hashing scheme `h1 + i * h2`.
fn index(h1: u64, h2: u64, i: u32, mask: u64) -> u64 {
    h1.wrapping_add(h2.wrapping_mul(i as u64)) & mask
}

fn splitmix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_false_negatives_after_add() {
        let mut b = BloomFilter::new(12, 7);
        let key: Vec<u8> = (0..20).collect();
        b.insert(&key);
        assert!(b.contains(&key), "added key must be present");
    }

    #[test]
    fn absent_keys_mostly_absent() {
        let mut b = BloomFilter::new(16, 7);
        for i in 0..1000u16 {
            b.insert(&i.to_be_bytes());
        }
        // A few thousand unseen keys should all be reported absent (no false
        // negatives means contains() only ever returns true for real adds or
        // rare false positives — here none expected at this size).
        let mut hits = 0;
        for i in 100_000..101_000u32 {
            if b.contains(&i.to_be_bytes()) {
                hits += 1;
            }
        }
        assert_eq!(hits, 0, "no false positives expected at this density");
    }

    #[test]
    fn shared_bloom_works() {
        let b = SharedBloom::new(12, 7);
        let key = b"shared-key";
        assert!(!b.contains(key), "must be absent before insert");
        b.insert(key);
        assert!(b.contains(key));
    }
}
