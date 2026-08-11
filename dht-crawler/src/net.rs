use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

/// A parsed CIDR block for IPv4 addresses. The peer hygiene filter keeps a
/// single-IP form (`10.0.0.1` = `/32`) so it can be loaded from a plain list.
#[derive(Debug, Clone, Copy)]
struct V4Net {
    base: u32,
    mask: u32,
}

impl V4Net {
    fn contains(&self, ip: u32) -> bool {
        ip & self.mask == self.base
    }
}

fn parse_v4_block(s: &str) -> Result<V4Net> {
    let (ip_str, prefix_str) = match s.split_once('/') {
        Some((a, b)) => (a, b),
        None => (s, "32"),
    };
    let ip: Ipv4Addr = ip_str
        .parse()
        .map_err(|_| anyhow!("invalid IPv4 address {ip_str:?}"))?;
    let prefix: u32 = prefix_str
        .parse()
        .map_err(|_| anyhow!("invalid prefix {prefix_str:?}"))?;
    if prefix > 32 {
        return Err(anyhow!("prefix out of range: {s:?}"));
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let base = u32::from(ip) & mask;
    Ok(V4Net { base, mask })
}

/// An immutable set of blocked IPs/CIDRs, loaded from a blocklist file.
///
/// The file format is one entry per line: an IPv4 address or CIDR block.
/// Blank lines and lines starting with `#` are ignored.
#[derive(Debug, Default, Clone)]
pub struct Blocklist {
    v4: Vec<V4Net>,
}

impl Blocklist {
    /// Load from `path`, or an empty blocklist if `None`.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            None => Ok(Self::default()),
            Some(p) => Self::from_file(p),
        }
    }

    /// Parse a blocklist file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("read blocklist {}", path.display()))?;
        Self::from_text(&text)
    }

    /// Parse blocklist text (one CIDR or IP per line; `#` comments allowed).
    pub fn from_text(text: &str) -> Result<Self> {
        let mut v4 = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            v4.push(parse_v4_block(line)?);
        }
        Ok(Self { v4 })
    }

    /// True if `ip` matches a blocked block.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let raw = u32::from(v4);
                self.v4.iter().any(|net| net.contains(raw))
            }
            IpAddr::V6(_) => false, // v6 peers are not filtered
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_blocklist_allows_all() {
        let b = Blocklist::default();
        assert!(!b.contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn cidr_matching() {
        let b = Blocklist::from_text(
            "10.0.0.0/8\n192.168.1.7\n203.0.113.0/24\n# comment\n\n",
        )
        .unwrap();
        assert!(b.contains("10.1.2.3".parse().unwrap()));
        assert!(b.contains("192.168.1.7".parse().unwrap()));
        assert!(!b.contains("192.168.1.8".parse().unwrap()));
        assert!(b.contains("203.0.113.255".parse().unwrap()));
        assert!(!b.contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn host_bits_are_masked() {
        // 192.168.5.1/24 must not match 192.168.6.1 even though low octet is 1.
        let b = Blocklist::from_text("192.168.5.0/24").unwrap();
        assert!(b.contains("192.168.5.200".parse().unwrap()));
        assert!(!b.contains("192.168.6.200".parse().unwrap()));
    }

    #[test]
    fn bad_lines_rejected() {
        assert!(Blocklist::from_text("not-an-ip\n").is_err());
        assert!(Blocklist::from_text("1.2.3.4/99\n").is_err());
    }
}
