//! CRC32C (Castagnoli polynomial 0x1EDC6F41) checksum.
//!
//! Used by BEP 42 (DHT security extension) and BEP 40 (canonical peer priority).

/// Precomputed CRC32C lookup table (Castagnoli polynomial, bit-reflected).
const CRC32C_TABLE: [u32; 256] = {
    const POLY: u32 = 0x82F6_3B78; // bit-reflected 0x1EDC6F41
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLY;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32C checksum of `data`.
#[must_use]
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_empty() {
        assert_eq!(crc32c(&[]), 0x0000_0000);
    }

    #[test]
    fn crc32c_known_value() {
        // CRC32C of "123456789" = 0xE3069283 (RFC 3720)
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }
}
