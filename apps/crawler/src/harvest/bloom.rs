use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const LN2: f64 = std::f64::consts::LN_2;

pub struct BloomFilter {
    bits: Vec<u64>,
    num_bits: usize,
    num_hashes: usize,
    inserted: usize,
}

impl BloomFilter {
    pub fn new(expected_items: usize, fp_rate: f64) -> Self {
        let num_bits = (-(expected_items as f64) * fp_rate.ln() / (LN2 * LN2)).ceil() as usize;
        let num_bits = num_bits.max(64);
        let num_hashes = ((num_bits as f64 / expected_items.max(1) as f64) * LN2).ceil() as usize;
        let num_hashes = num_hashes.clamp(1, 16);
        BloomFilter {
            bits: vec![0; num_bits.div_ceil(64)],
            num_bits,
            num_hashes,
            inserted: 0,
        }
    }

    #[cfg(test)]
    pub fn with_size(num_bits: usize, num_hashes: usize) -> Self {
        BloomFilter {
            bits: vec![0; num_bits.div_ceil(64)],
            num_bits: num_bits.max(64),
            num_hashes: num_hashes.clamp(1, 16),
            inserted: 0,
        }
    }

    fn hashes(&self, data: &[u8]) -> (u64, u64) {
        let h1 = hash_seeded(0x9E37_79B9_7F4A_7C15, data);
        let h2 = hash_seeded(0xC2B2_AE3D_27D4_EB4F, data);
        (h1, h2)
    }

    fn idx(&self, h1: u64, h2: u64, i: usize) -> usize {
        let h = h1.wrapping_add((i as u64).wrapping_mul(h2));
        (h % self.num_bits as u64) as usize
    }

    pub fn insert(&mut self, data: &[u8]) {
        let (h1, h2) = self.hashes(data);
        for i in 0..self.num_hashes {
            let idx = self.idx(h1, h2, i);
            self.bits[idx / 64] |= 1 << (idx % 64);
        }
        self.inserted += 1;
    }

    pub fn contains(&self, data: &[u8]) -> bool {
        let (h1, h2) = self.hashes(data);
        for i in 0..self.num_hashes {
            let idx = self.idx(h1, h2, i);
            if self.bits[idx / 64] & (1 << (idx % 64)) == 0 {
                return false;
            }
        }
        true
    }

    pub fn inserted(&self) -> usize {
        self.inserted
    }

    pub fn clear(&mut self) {
        self.bits.iter_mut().for_each(|w| *w = 0);
        self.inserted = 0;
    }
}

fn hash_seeded(seed: u64, data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let mut b = BloomFilter::with_size(1024, 4);
        b.insert(b"hello");
        assert!(b.contains(b"hello"));
        assert!(!b.contains(b"world"));
    }

    #[test]
    fn no_false_negative_many() {
        let mut b = BloomFilter::with_size(65536, 6);
        let items: Vec<Vec<u8>> = (0..1000).map(|i| format!("ih-{i}").into_bytes()).collect();
        for it in &items {
            b.insert(it);
        }
        for it in &items {
            assert!(b.contains(it));
        }
    }
}
