use crate::krpc::Infohash;
use sha1::{Digest, Sha1};

pub fn check(info_hash: &Infohash, metadata: &[u8]) -> bool {
    let digest = Sha1::digest(metadata);
    digest.as_slice() == info_hash.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_matches_sha1() {
        let meta = b"d4:infod6:lengthi42e4:name4:testee";
        let mut ih = [0u8; 20];
        ih.copy_from_slice(Sha1::digest(meta).as_slice());
        assert!(check(&ih, meta));
        let bad = b"d4:infod6:lengthi43e4:name4:testee";
        assert!(!check(&ih, bad));
    }
}
