use bytes::{Buf, BufMut, Bytes, BytesMut};

use gaia_core::Id20;

use crate::error::{Error, Result};

/// `BitTorrent` protocol string.
const PROTOCOL: &[u8] = b"BitTorrent protocol";

/// Total handshake size: 1 + 19 + 8 + 20 + 20 = 68 bytes.
pub const HANDSHAKE_SIZE: usize = 68;

/// Peer wire handshake (BEP 3).
///
/// Format: `<pstrlen><pstr><reserved><info_hash><peer_id>`
/// - pstrlen: 1 byte (19)
/// - pstr: 19 bytes ("`BitTorrent` protocol")
/// - reserved: 8 bytes (extension flags)
/// - `info_hash`: 20 bytes
/// - `peer_id`: 20 bytes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// 8-byte reserved field for extension flags.
    pub reserved: [u8; 8],
    /// Info hash of the torrent.
    pub info_hash: Id20,
    /// Peer ID.
    pub peer_id: Id20,
}

impl Handshake {
    /// Create a new handshake with extension protocol support (BEP 10).
    #[must_use]
    pub fn new(info_hash: Id20, peer_id: Id20) -> Self {
        let mut reserved = [0u8; 8];
        // Bit 20 (byte 5, bit 4) = Extension Protocol (BEP 10)
        reserved[5] |= 0x10;
        Self {
            reserved,
            info_hash,
            peer_id,
        }
    }

    /// Check if the peer supports the Extension Protocol (BEP 10).
    #[must_use]
    pub fn supports_extensions(&self) -> bool {
        self.reserved[5] & 0x10 != 0
    }

    /// Check if the peer supports DHT (BEP 5).
    #[must_use]
    pub fn supports_dht(&self) -> bool {
        self.reserved[7] & 0x01 != 0
    }

    /// Enable DHT support flag.
    #[must_use]
    pub fn with_dht(mut self) -> Self {
        self.reserved[7] |= 0x01;
        self
    }

    /// Check if the peer supports Fast Extension (BEP 6).
    #[must_use]
    pub fn supports_fast(&self) -> bool {
        self.reserved[7] & 0x04 != 0
    }

    /// Enable Fast Extension flag.
    #[must_use]
    pub fn with_fast(mut self) -> Self {
        self.reserved[7] |= 0x04;
        self
    }

    /// Serialize to bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(HANDSHAKE_SIZE);
        buf.put_u8(19);
        buf.put_slice(PROTOCOL);
        buf.put_slice(&self.reserved);
        buf.put_slice(self.info_hash.as_bytes());
        buf.put_slice(self.peer_id.as_bytes());
        buf.freeze()
    }

    /// Parse from bytes. Input must be exactly 68 bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too short or has an invalid protocol string.
    pub fn from_bytes(mut data: &[u8]) -> Result<Self> {
        if data.len() < HANDSHAKE_SIZE {
            return Err(Error::InvalidHandshake(format!(
                "need {} bytes, got {}",
                HANDSHAKE_SIZE,
                data.len()
            )));
        }

        let pstrlen = data.get_u8();
        if pstrlen != 19 {
            return Err(Error::InvalidHandshake(format!(
                "pstrlen {pstrlen}, expected 19"
            )));
        }

        let pstr = &data[..19];
        if pstr != PROTOCOL {
            return Err(Error::InvalidHandshake("wrong protocol string".into()));
        }
        data.advance(19);

        let mut reserved = [0u8; 8];
        reserved.copy_from_slice(&data[..8]);
        data.advance(8);

        let info_hash =
            Id20::from_bytes(&data[..20]).map_err(|e| Error::InvalidHandshake(e.to_string()))?;
        data.advance(20);

        let peer_id =
            Id20::from_bytes(&data[..20]).map_err(|e| Error::InvalidHandshake(e.to_string()))?;

        Ok(Self {
            reserved,
            info_hash,
            peer_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_round_trip() {
        let info_hash = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        let peer_id = Id20::from_hex("0102030405060708091011121314151617181920").unwrap();

        let hs = Handshake::new(info_hash, peer_id);
        assert!(hs.supports_extensions());

        let bytes = hs.to_bytes();
        assert_eq!(bytes.len(), HANDSHAKE_SIZE);

        let parsed = Handshake::from_bytes(&bytes).unwrap();
        assert_eq!(hs, parsed);
    }

    #[test]
    fn handshake_dht_flag() {
        let hs = Handshake::new(Id20::ZERO, Id20::ZERO).with_dht();
        assert!(hs.supports_dht());
        assert!(hs.supports_extensions());

        let parsed = Handshake::from_bytes(&hs.to_bytes()).unwrap();
        assert!(parsed.supports_dht());
    }

    /// BEP 10: extension protocol is signalled by bit 20 from the right in the
    /// 8-byte reserved field. Bit 20 maps to byte index 5, bit 4 (0x10).
    #[test]
    fn ext_handshake_reserved_bit_position() {
        // Construct expected reserved bytes with ONLY bit 20 set
        let mut expected_reserved = [0u8; 8];
        // Bit 20 from the right: byte index = 20 / 8 = 2 from the right = index 5,
        // bit position = 20 % 8 = 4, so mask = 0x10.
        expected_reserved[5] = 0x10;

        // Verify the Handshake::new() constructor sets exactly this bit
        let hs = Handshake::new(Id20::ZERO, Id20::ZERO);
        assert_eq!(
            hs.reserved, expected_reserved,
            "Handshake::new() reserved field must match BEP 10 spec"
        );
        assert_eq!(
            hs.reserved[5] & 0x10,
            0x10,
            "BEP 10 extension bit must be at reserved[5] & 0x10"
        );
        assert!(hs.supports_extensions());

        // Verify no other reserved bytes are set by default (only BEP 10)
        for (i, &byte) in hs.reserved.iter().enumerate() {
            if i == 5 {
                assert_eq!(byte, 0x10, "byte 5 should be exactly 0x10");
            } else {
                assert_eq!(byte, 0, "byte {i} should be zero when only BEP 10 is set");
            }
        }

        // Verify that clearing byte 5 bit 4 disables extension support
        let mut hs_no_ext = hs;
        hs_no_ext.reserved[5] &= !0x10;
        assert!(!hs_no_ext.supports_extensions());

        // Verify bit 20 interpretation: counting from bit 0 (rightmost of reserved[7])
        // bit 20 = byte 5 (from left, 0-indexed), bit 4 within that byte
        let bit_index = 20;
        let byte_index_from_right = bit_index / 8; // = 2
        let byte_index = 7 - byte_index_from_right; // = 5
        let bit_within_byte = bit_index % 8; // = 4
        assert_eq!(byte_index, 5);
        assert_eq!(bit_within_byte, 4);
        assert_eq!(1u8 << bit_within_byte, 0x10);
    }

    #[test]
    fn handshake_too_short() {
        assert!(Handshake::from_bytes(&[0u8; 10]).is_err());
    }

    #[test]
    fn handshake_fast_flag() {
        let hs = Handshake::new(Id20::ZERO, Id20::ZERO).with_fast();
        assert!(hs.supports_fast());
        assert!(hs.supports_extensions());
        // Ensure DHT and fast don't interfere
        let hs2 = hs.with_dht();
        assert!(hs2.supports_fast());
        assert!(hs2.supports_dht());

        let parsed = Handshake::from_bytes(&hs2.to_bytes()).unwrap();
        assert!(parsed.supports_fast());
        assert!(parsed.supports_dht());
    }
}
