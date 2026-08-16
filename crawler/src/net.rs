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

/// True if `ip` is globally dialable in practice. Peer-reported addresses from
/// DHT announce/get_peers are often the sender's own (NAT'd) address: private
/// RFC 1918 space, loopback, and carrier-grade NAT (100.64.0.0/10) can never be
/// reached from this host, so dialing them is guaranteed waste. `--no-restrict-ips`
/// disables *routing-table* diversity restrictions, but a hint dial to a
/// non-routable address always fails — it is filtered here unconditionally.
pub fn is_globally_dialable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V6(v6) => {
            // Globally routable unicast: not loopback, link-local, ULA, or
            // multicast.
            !v6.is_loopback()
                && !v6.is_multicast()
                && !v6.is_unspecified()
                && !(v6.segments()[0] & 0xfe00 == 0xfc00) // fc00::/7 ULA
                && !(v6.segments()[0] & 0xffc0 == 0xfe80) // fe80::/10 link-local
        }
        IpAddr::V4(v4) => {
            let o = v4.octets();
            let raw = u32::from(v4);
            // 0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10 (CGNAT), 127.0.0.0/8,
            // 169.254.0.0/16 (link-local), 172.16.0.0/12, 192.168.0.0/16,
            // 224.0.0.0/4 (multicast), 240.0.0.0/4 (reserved).
            !(o[0] == 0)
                && !(o[0] == 10)
                && !(o[0] == 100 && (o[1] & 0xc0) == 0x40)
                && !(o[0] == 127)
                && !(o[0] == 169 && o[1] == 254)
                && !(o[0] == 172 && (o[1] & 0xf0) == 16)
                && !(o[0] == 192 && o[1] == 168)
                && !(0xE0000000..0xF0000000).contains(&raw)
                && !(raw >= 0xF0000000)
        }
    }
}

/// In-run cache of peer IPs that repeatedly failed to connect. Once an IP has
/// failed `required_failures` times within the TTL window, it is skipped when
/// dialing further hashes; it becomes eligible again after the TTL elapses.
#[derive(Debug)]
pub struct DeadPeerCache {
    /// How many connect failures an IP needs before it is skipped.
    required_failures: usize,
    /// How long a dead marking lasts before the IP is retried.
    ttl_secs: i64,
    /// IP → (failure count, last-failure unix time).
    entries: std::collections::HashMap<IpAddr, (u32, i64)>,
}

impl DeadPeerCache {
    /// Failures required to mark dead, and the TTL in seconds.
    pub fn new(required_failures: usize, ttl_secs: i64) -> Self {
        Self {
            required_failures: required_failures.max(1),
            ttl_secs: ttl_secs.max(1),
            entries: std::collections::HashMap::new(),
        }
    }

    /// True if `ip` should be skipped (dead within the TTL window).
    pub fn is_dead(&self, ip: IpAddr, now: i64) -> bool {
        self.entries.get(&ip).is_some_and(|(count, last)| {
            *count >= self.required_failures as u32 && now - *last < self.ttl_secs
        })
    }

    /// Record a connect failure for `ip`. Returns true if it just became dead.
    pub fn record_failure(&mut self, ip: IpAddr, now: i64) -> bool {
        let entry = self.entries.entry(ip).or_insert((0, now));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = now;
        entry.0 >= self.required_failures as u32
    }

    /// Forget expired entries; keeps the map small.
    pub fn prune(&mut self, now: i64) {
        self.entries
            .retain(|_, (_, last)| now - *last < self.ttl_secs);
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

    #[test]
    fn global_dialability_classifies_private_and_cgnat() {
        let cases: &[(&str, bool)] = &[
            ("8.8.8.8", true),
            ("93.184.216.34", true),
            ("1.2.3.4", true),
            ("10.1.2.3", false),           // RFC1918
            ("172.16.0.1", false),         // RFC1918
            ("172.31.255.254", false),     // RFC1918
            ("192.168.1.7", false),        // RFC1918
            ("100.64.0.1", false),         // CGNAT
            ("100.127.255.254", false),    // CGNAT
            ("127.0.0.1", false),          // loopback
            ("169.254.169.254", false),    // link-local
            ("0.0.0.0", false),            // unspecified
            ("224.0.0.1", false),          // multicast
            ("240.0.0.1", false),          // reserved
            ("2001:db8::1", true),         // docs /2000::/3 globally unique pattern
            ("::1", false),                // loopback v6
            ("fc00::1", false),            // ULA
            ("fd12:3456::1", false),       // ULA
            ("fe80::1", false),            // link-local v6
            ("2001:4860:4860::8888", true), // Google DNS v6
        ];
        for (s, expect) in cases {
            let ip: IpAddr = s.parse().unwrap();
            assert_eq!(is_globally_dialable(ip), *expect, "case {s}");
        }
    }

    #[test]
    fn dead_peer_cache_skips_after_threshold_and_expires() {
        let mut cache = DeadPeerCache::new(2, 600);
        let ip: IpAddr = "93.184.216.34".parse().unwrap();
        let now = 1_700_000_000i64;

        assert!(!cache.is_dead(ip, now));
        cache.record_failure(ip, now);
        assert!(!cache.is_dead(ip, now), "one failure is below threshold");
        cache.record_failure(ip, now);
        assert!(cache.is_dead(ip, now), "two failures mark dead");

        assert!(
            cache.is_dead(ip, now + 300),
            "still dead inside TTL window"
        );
        assert!(
            !cache.is_dead(ip, now + 601),
            "expired after TTL"
        );

        cache.record_failure(ip, now);
        cache.prune(now + 601);
        assert!(
            !cache.is_dead(ip, now + 601),
            "prune removes expired entries"
        );
    }

    #[test]
    fn dead_peer_cache_threshold_one() {
        let mut cache = DeadPeerCache::new(1, 60);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        let now = 1_700_000_000i64;
        assert!(cache.record_failure(ip, now), "single failure marks dead");
        assert!(cache.is_dead(ip, now));
    }
}
