//! BEP 44: DHT arbitrary data storage types and validation.
//!
//! Defines item types, signature construction, and validation for both
//! immutable (SHA-1-keyed) and mutable (ed25519-signed) DHT items.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use gaia_core::{Id20, sha1};

use crate::error::{Error, Result};

/// Maximum size of a bencoded value in a BEP 44 item (bytes).
pub const MAX_VALUE_SIZE: usize = 1000;

/// Maximum size of the salt field (bytes).
pub const MAX_SALT_SIZE: usize = 64;

/// An immutable DHT item. Key = SHA-1(bencoded value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableItem {
    /// The bencoded value (max 1000 bytes).
    pub value: Vec<u8>,
    /// SHA-1 hash of `value`, used as the lookup key / target.
    pub target: Id20,
}

impl ImmutableItem {
    /// Create an immutable item from raw bencoded bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if value exceeds 1000 bytes.
    pub fn new(value: Vec<u8>) -> Result<Self> {
        if value.len() > MAX_VALUE_SIZE {
            return Err(Error::Bep44ValueTooLarge {
                size: value.len(),
                max: MAX_VALUE_SIZE,
            });
        }
        let target = sha1(&value);
        Ok(Self { value, target })
    }

    /// Validate that the value matches the expected target hash.
    #[must_use]
    pub fn verify(&self) -> bool {
        sha1(&self.value) == self.target
    }
}

/// A mutable DHT item. Key = ed25519 public key + optional salt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutableItem {
    /// The bencoded value (max 1000 bytes).
    pub value: Vec<u8>,
    /// Ed25519 public key (32 bytes).
    pub public_key: [u8; 32],
    /// Ed25519 signature (64 bytes) over the signing buffer.
    pub signature: [u8; 64],
    /// Monotonically increasing sequence number.
    pub seq: i64,
    /// Optional salt (max 64 bytes). Empty vec = no salt.
    pub salt: Vec<u8>,
    /// Derived target: SHA-1(public_key + salt).
    pub target: Id20,
}

impl MutableItem {
    /// Create a mutable item and sign it with the given keypair.
    ///
    /// - `keypair`: ed25519 signing key (contains both secret and public key)
    /// - `value`: raw bencoded value (max 1000 bytes)
    /// - `seq`: sequence number (must increase with each update)
    /// - `salt`: optional salt (max 64 bytes, empty vec for none)
    ///
    /// # Errors
    ///
    /// Returns an error if value exceeds 1000 bytes or salt exceeds 64 bytes.
    pub fn create(keypair: &SigningKey, value: Vec<u8>, seq: i64, salt: Vec<u8>) -> Result<Self> {
        if value.len() > MAX_VALUE_SIZE {
            return Err(Error::Bep44ValueTooLarge {
                size: value.len(),
                max: MAX_VALUE_SIZE,
            });
        }
        if salt.len() > MAX_SALT_SIZE {
            return Err(Error::Bep44SaltTooLarge {
                size: salt.len(),
                max: MAX_SALT_SIZE,
            });
        }

        let public_key = keypair.verifying_key().to_bytes();
        let target = compute_mutable_target(&public_key, &salt);
        let sign_buf = build_signing_buffer(&salt, seq, &value);
        let signature = keypair.sign(&sign_buf);

        Ok(Self {
            value,
            public_key,
            signature: signature.to_bytes(),
            seq,
            salt,
            target,
        })
    }

    /// Verify the ed25519 signature against the public key and signing buffer.
    #[must_use]
    pub fn verify(&self) -> bool {
        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.public_key) else {
            return false;
        };
        // Gap fix: Signature::from_bytes is infallible in ed25519-dalek v2
        let signature = Signature::from_bytes(&self.signature);
        let sign_buf = build_signing_buffer(&self.salt, self.seq, &self.value);
        verifying_key.verify(&sign_buf, &signature).is_ok()
    }

    /// Check that the target matches SHA-1(public_key + salt).
    #[must_use]
    pub fn verify_target(&self) -> bool {
        compute_mutable_target(&self.public_key, &self.salt) == self.target
    }
}

/// Compute the DHT target for a mutable item: `SHA-1(public_key + salt)`.
#[must_use]
pub fn compute_mutable_target(public_key: &[u8; 32], salt: &[u8]) -> Id20 {
    let mut buf = Vec::with_capacity(32 + salt.len());
    buf.extend_from_slice(public_key);
    buf.extend_from_slice(salt);
    sha1(&buf)
}

/// Build the byte buffer that gets signed/verified for mutable items.
///
/// Without salt: `3:seqi{seq}e1:v{len}:{value}`
/// With salt: `4:salt{salt_len}:{salt}3:seqi{seq}e1:v{value_len}:{value}`
#[must_use]
pub fn build_signing_buffer(salt: &[u8], seq: i64, value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + value.len() + salt.len());

    if !salt.is_empty() {
        // "4:salt{len}:{salt}"
        buf.extend_from_slice(b"4:salt");
        buf.extend_from_slice(salt.len().to_string().as_bytes());
        buf.push(b':');
        buf.extend_from_slice(salt);
    }

    // "3:seqi{seq}e"
    buf.extend_from_slice(b"3:seqi");
    buf.extend_from_slice(seq.to_string().as_bytes());
    buf.push(b'e');

    // "1:v{len}:{value}"
    buf.extend_from_slice(b"1:v");
    buf.extend_from_slice(value.len().to_string().as_bytes());
    buf.push(b':');
    buf.extend_from_slice(value);

    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn immutable_item_hash_is_sha1_of_value() {
        let value = b"12:Hello World!".to_vec(); // bencoded string
        let item = ImmutableItem::new(value.clone()).unwrap();
        assert_eq!(item.target, sha1(&value));
        assert!(item.verify());
    }

    #[test]
    fn immutable_item_rejects_oversized_value() {
        let value = vec![0u8; MAX_VALUE_SIZE + 1];
        let err = ImmutableItem::new(value).unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    #[test]
    fn immutable_item_verify_detects_corruption() {
        let mut item = ImmutableItem::new(b"5:hello".to_vec()).unwrap();
        item.value[0] = 0xFF; // corrupt
        assert!(!item.verify());
    }

    #[test]
    fn mutable_item_sign_and_verify() {
        let keypair = SigningKey::from_bytes(&[1u8; 32]);
        let value = b"12:Hello World!".to_vec();
        let item = MutableItem::create(&keypair, value, 1, Vec::new()).unwrap();
        assert!(item.verify());
        assert!(item.verify_target());
    }

    #[test]
    fn mutable_item_with_salt() {
        let keypair = SigningKey::from_bytes(&[2u8; 32]);
        let value = b"4:test".to_vec();
        let salt = b"my-salt".to_vec();
        let item = MutableItem::create(&keypair, value, 42, salt.clone()).unwrap();
        assert!(item.verify());
        assert!(item.verify_target());
        assert_eq!(item.salt, salt);
    }

    #[test]
    fn mutable_item_rejects_oversized_value() {
        let keypair = SigningKey::from_bytes(&[3u8; 32]);
        let value = vec![0u8; MAX_VALUE_SIZE + 1];
        let err = MutableItem::create(&keypair, value, 1, Vec::new()).unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    #[test]
    fn mutable_item_rejects_oversized_salt() {
        let keypair = SigningKey::from_bytes(&[4u8; 32]);
        let value = b"4:test".to_vec();
        let salt = vec![0u8; MAX_SALT_SIZE + 1];
        let err = MutableItem::create(&keypair, value, 1, salt).unwrap_err();
        assert!(err.to_string().contains("salt"));
    }

    #[test]
    fn mutable_item_verify_fails_on_wrong_key() {
        let keypair = SigningKey::from_bytes(&[5u8; 32]);
        let value = b"4:test".to_vec();
        let mut item = MutableItem::create(&keypair, value, 1, Vec::new()).unwrap();
        // Replace public key with a different one
        item.public_key = SigningKey::from_bytes(&[6u8; 32])
            .verifying_key()
            .to_bytes();
        assert!(!item.verify());
    }

    #[test]
    fn mutable_item_verify_fails_on_tampered_value() {
        let keypair = SigningKey::from_bytes(&[7u8; 32]);
        let value = b"4:test".to_vec();
        let mut item = MutableItem::create(&keypair, value, 1, Vec::new()).unwrap();
        item.value = b"5:tampr".to_vec();
        assert!(!item.verify());
    }

    #[test]
    fn mutable_target_is_sha1_of_pubkey_plus_salt() {
        let pubkey = [0xABu8; 32];
        let salt = b"hello";
        let target = compute_mutable_target(&pubkey, salt);

        let mut expected_buf = Vec::new();
        expected_buf.extend_from_slice(&pubkey);
        expected_buf.extend_from_slice(salt);
        assert_eq!(target, sha1(&expected_buf));
    }

    #[test]
    fn mutable_target_no_salt() {
        let pubkey = [0xCDu8; 32];
        let target = compute_mutable_target(&pubkey, &[]);
        // With no salt, target = SHA-1(pubkey)
        assert_eq!(target, sha1(&pubkey));
    }

    #[test]
    fn signing_buffer_no_salt() {
        let buf = build_signing_buffer(&[], 1, b"12:Hello World!");
        assert_eq!(buf, b"3:seqi1e1:v15:12:Hello World!");
    }

    #[test]
    fn signing_buffer_with_salt() {
        let buf = build_signing_buffer(b"foobar", 4, b"12:Hello World!");
        assert_eq!(buf, b"4:salt6:foobar3:seqi4e1:v15:12:Hello World!");
    }

    #[test]
    fn signing_buffer_negative_seq() {
        let buf = build_signing_buffer(&[], -1, b"1:x");
        assert_eq!(buf, b"3:seqi-1e1:v3:1:x");
    }

    #[test]
    fn sequence_number_monotonicity() {
        let keypair = SigningKey::from_bytes(&[8u8; 32]);
        let item1 = MutableItem::create(&keypair, b"4:aaaa".to_vec(), 1, Vec::new()).unwrap();
        let item2 = MutableItem::create(&keypair, b"4:bbbb".to_vec(), 2, Vec::new()).unwrap();
        assert!(item2.seq > item1.seq);
        assert!(item1.verify());
        assert!(item2.verify());
    }

    // ---- BEP 44 spec test vectors (https://www.bittorrent.org/beps/bep_0044.html) ----
    //
    // The spec provides exact keypair, targets, and test conditions.
    // Private key is a 64-byte expanded ed25519 key (SHA-512 output of seed),
    // requiring the hazmat API to sign.

    /// BEP 44 spec public key (32 bytes).
    const BEP44_PUBLIC_KEY: [u8; 32] = [
        0x77, 0xff, 0x84, 0x90, 0x5a, 0x91, 0x93, 0x63, 0x67, 0xc0, 0x13, 0x60, 0x80, 0x31, 0x04,
        0xf9, 0x24, 0x32, 0xfc, 0xd9, 0x04, 0xa4, 0x35, 0x11, 0x87, 0x6d, 0xf5, 0xcd, 0xf3, 0xe7,
        0xe5, 0x48,
    ];

    /// BEP 44 spec expanded private key (64 bytes).
    const BEP44_EXPANDED_KEY: [u8; 64] = [
        0xe0, 0x6d, 0x31, 0x83, 0xd1, 0x41, 0x59, 0x22, 0x84, 0x33, 0xed, 0x59, 0x92, 0x21, 0xb8,
        0x0b, 0xd0, 0xa5, 0xce, 0x83, 0x52, 0xe4, 0xbd, 0xf0, 0x26, 0x2f, 0x76, 0x78, 0x6e, 0xf1,
        0xc7, 0x4d, 0xb7, 0xe7, 0xa9, 0xfe, 0xa2, 0xc0, 0xeb, 0x26, 0x9d, 0x61, 0xe3, 0xb3, 0x8e,
        0x45, 0x0a, 0x22, 0xe7, 0x54, 0x94, 0x1a, 0xc7, 0x84, 0x79, 0xd6, 0xc5, 0x4e, 0x1f, 0xaf,
        0x60, 0x37, 0x88, 0x1d,
    ];

    /// BEP 44 spec bencoded value: "Hello World!" as bencode string.
    const BEP44_VALUE: &[u8] = b"12:Hello World!";

    /// Sign a message with the BEP 44 spec expanded key using hazmat API.
    fn bep44_spec_sign(message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::hazmat::{ExpandedSecretKey, raw_sign};
        use sha2::Sha512;

        let esk = ExpandedSecretKey::from_bytes(&BEP44_EXPANDED_KEY);
        let vk = VerifyingKey::from_bytes(&BEP44_PUBLIC_KEY).unwrap();
        let sig = raw_sign::<Sha512>(&esk, message, &vk);
        sig.to_bytes()
    }

    #[test]
    fn bep44_mutable_no_salt_spec_vector() {
        // BEP 44 Vector 1: mutable, no salt
        // target = 4a533d47ec9c7d95b1ad75f576cffc641853b750
        let expected_target = Id20::from_hex("4a533d47ec9c7d95b1ad75f576cffc641853b750").unwrap();

        let value = BEP44_VALUE.to_vec();
        let seq: i64 = 1;
        let salt = Vec::new();

        // Compute target
        let target = compute_mutable_target(&BEP44_PUBLIC_KEY, &salt);
        assert_eq!(target, expected_target, "mutable target (no salt) mismatch");

        // Sign with spec expanded key via hazmat
        let sign_buf = build_signing_buffer(&salt, seq, &value);
        let sig_bytes = bep44_spec_sign(&sign_buf);

        // Construct MutableItem and verify signature
        let item = MutableItem {
            value,
            public_key: BEP44_PUBLIC_KEY,
            signature: sig_bytes,
            seq,
            salt,
            target,
        };
        assert!(item.verify(), "signature verification failed");
        assert!(item.verify_target(), "target verification failed");
    }

    #[test]
    fn bep44_mutable_salted_spec_vector() {
        // BEP 44 Vector 2: mutable, salt "foobar"
        // target = 411eba73b6f087ca51a3795d9c8c938d365e32c1
        let expected_target = Id20::from_hex("411eba73b6f087ca51a3795d9c8c938d365e32c1").unwrap();

        let value = BEP44_VALUE.to_vec();
        let seq: i64 = 1;
        let salt = b"foobar".to_vec();

        // Compute target
        let target = compute_mutable_target(&BEP44_PUBLIC_KEY, &salt);
        assert_eq!(target, expected_target, "mutable target (salted) mismatch");

        // Sign with spec expanded key via hazmat
        let sign_buf = build_signing_buffer(&salt, seq, &value);
        let sig_bytes = bep44_spec_sign(&sign_buf);

        // Construct MutableItem and verify signature
        let item = MutableItem {
            value,
            public_key: BEP44_PUBLIC_KEY,
            signature: sig_bytes,
            seq,
            salt,
            target,
        };
        assert!(item.verify(), "signature verification failed");
        assert!(item.verify_target(), "target verification failed");
    }

    #[test]
    fn bep44_immutable_spec_vector() {
        // BEP 44 Vector 3: immutable
        // target = e5f96f6f38320f0f33959cb4d3d656452117aadb
        let expected_target = Id20::from_hex("e5f96f6f38320f0f33959cb4d3d656452117aadb").unwrap();

        let item = ImmutableItem::new(BEP44_VALUE.to_vec()).unwrap();
        assert_eq!(item.target, expected_target, "immutable target mismatch");
        assert!(item.verify(), "immutable item verification failed");
    }

    #[test]
    fn bep44_signing_buffer_construction() {
        // Verify exact bencoded signing buffer bytes for vector 1 (no salt, seq=1).
        // Per BEP 44: "3:seqi{seq}e1:v{len}:{value}"
        // = "3:seqi1e1:v15:12:Hello World!"
        let buf = build_signing_buffer(&[], 1, BEP44_VALUE);
        let expected = b"3:seqi1e1:v15:12:Hello World!";
        assert_eq!(buf, expected, "signing buffer mismatch for vector 1");

        // Also verify salted buffer for vector 2:
        // "4:salt6:foobar3:seqi1e1:v15:12:Hello World!"
        let buf_salted = build_signing_buffer(b"foobar", 1, BEP44_VALUE);
        let expected_salted = b"4:salt6:foobar3:seqi1e1:v15:12:Hello World!";
        assert_eq!(
            buf_salted, expected_salted,
            "signing buffer mismatch for vector 2"
        );
    }
}
