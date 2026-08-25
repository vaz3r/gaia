use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use std::net::IpAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha1 = Hmac<Sha1>;

pub const TOKEN_LEN: usize = 20;

#[derive(Clone)]
pub struct TokenGenerator {
    secret: [u8; 32],
    prev_secret: [u8; 32],
    window: Duration,
}

impl TokenGenerator {
    pub fn new(secret: [u8; 32], window: Duration) -> Self {
        TokenGenerator {
            secret,
            prev_secret: secret,
            window,
        }
    }

    pub fn rotate(&mut self, new_secret: [u8; 32]) {
        self.prev_secret = self.secret;
        self.secret = new_secret;
    }

    fn epoch(now: u64, window: u64) -> u64 {
        now / window
    }

    fn window_secs(&self) -> u64 {
        let secs = self.window.as_secs();
        if secs == 0 { 1 } else { secs }
    }

    pub fn generate(&self, ip: IpAddr) -> [u8; TOKEN_LEN] {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let epoch = Self::epoch(now, self.window_secs());
        self.sign(&self.secret, ip, epoch)
    }

    pub fn verify(&self, ip: IpAddr, token: &[u8]) -> bool {
        if token.len() != TOKEN_LEN {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let epoch = Self::epoch(now, self.window_secs());
        let mut tok = [0u8; TOKEN_LEN];
        tok.copy_from_slice(token);
        self.sign(&self.secret, ip, epoch) == tok
            || self.sign(&self.secret, ip, epoch.wrapping_sub(1)) == tok
            || self.sign(&self.prev_secret, ip, epoch) == tok
            || self.sign(&self.prev_secret, ip, epoch.wrapping_sub(1)) == tok
    }

    fn sign(&self, secret: &[u8; 32], ip: IpAddr, epoch: u64) -> [u8; TOKEN_LEN] {
        let mut mac = HmacSha1::new_from_slice(secret).expect("hmac accepts any key length");
        match ip {
            IpAddr::V4(v4) => mac.update(&v4.octets()),
            IpAddr::V6(v6) => mac.update(&v6.octets()),
        }
        mac.update(&epoch.to_be_bytes());
        let out = mac.finalize().into_bytes();
        let mut tok = [0u8; TOKEN_LEN];
        tok.copy_from_slice(out.as_ref());
        tok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip() {
        let tg = TokenGenerator::new([7u8; 32], Duration::from_secs(300));
        let ip: IpAddr = "124.31.75.21".parse().unwrap();
        let tok = tg.generate(ip);
        assert!(tg.verify(ip, &tok));
        assert!(!tg.verify(ip, &[0u8; TOKEN_LEN]));
        let other: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!tg.verify(other, &tok));
    }

    #[test]
    fn rotate_overlap() {
        let mut tg = TokenGenerator::new([1u8; 32], Duration::from_secs(300));
        let ip: IpAddr = "124.31.75.21".parse().unwrap();
        let old = tg.generate(ip);
        tg.rotate([2u8; 32]);
        let new = tg.generate(ip);
        assert!(tg.verify(ip, &new));
        assert!(tg.verify(ip, &old));
    }
}
