use dashmap::DashMap;
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cidr {
    V4 { net: u32, mask: u32 },
    V6 { net: u128, mask: u128 },
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let (ip_str, prefix_str) = s
            .split_once('/')
            .ok_or_else(|| format!("missing '/' in CIDR string '{s}'"))?;
        let prefix: u8 = prefix_str
            .trim()
            .parse()
            .map_err(|e| format!("invalid prefix in '{s}': {e}"))?;
        let ip: IpAddr = ip_str
            .trim()
            .parse()
            .map_err(|e| format!("invalid IP in '{s}': {e}"))?;
        match ip {
            IpAddr::V4(v4) => {
                if prefix > 32 {
                    return Err(format!("prefix {prefix} > 32 for IPv4 in '{s}'"));
                }
                let mask = if prefix == 0 {
                    0
                } else {
                    !0u32 << (32 - prefix)
                };
                let net = u32::from_be_bytes(v4.octets()) & mask;
                Ok(Cidr::V4 { net, mask })
            }
            IpAddr::V6(v6) => {
                if prefix > 128 {
                    return Err(format!("prefix {prefix} > 128 for IPv6 in '{s}'"));
                }
                let mask = if prefix == 0 {
                    0
                } else {
                    !0u128 << (128 - prefix)
                };
                let net = u128::from_be_bytes(v6.octets()) & mask;
                Ok(Cidr::V6 { net, mask })
            }
        }
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self, ip) {
            (Cidr::V4 { net, mask }, IpAddr::V4(v4)) => {
                let ip_u32 = u32::from_be_bytes(v4.octets());
                (ip_u32 & mask) == *net
            }
            (Cidr::V6 { net, mask }, IpAddr::V6(v6)) => {
                let ip_u128 = u128::from_be_bytes(v6.octets());
                (ip_u128 & mask) == *net
            }
            _ => false,
        }
    }
}

pub struct IpCooldownCache {
    quarantined: DashMap<IpAddr, Instant>,
    cooldown_duration: Duration,
    max_entries: usize,
    blacklist_ips: HashSet<IpAddr>,
    blacklist_cidrs: Vec<Cidr>,
}

impl IpCooldownCache {
    pub fn new(
        cooldown_duration: Duration,
        max_entries: usize,
        blacklist_path: Option<&Path>,
    ) -> Self {
        let mut blacklist_ips = HashSet::new();
        let mut blacklist_cidrs = Vec::new();

        if let Some(path) = blacklist_path {
            if path.exists() {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if trimmed.is_empty() || trimmed.starts_with('#') {
                                continue;
                            }
                            let entry = trimmed.split('#').next().unwrap_or("").trim();
                            if entry.contains('/') {
                                match Cidr::parse(entry) {
                                    Ok(cidr) => blacklist_cidrs.push(cidr),
                                    Err(err) => tracing::warn!(entry, error = %err, "invalid CIDR in blacklist"),
                                }
                            } else {
                                match entry.parse::<IpAddr>() {
                                    Ok(ip) => {
                                        blacklist_ips.insert(ip);
                                    }
                                    Err(err) => tracing::warn!(entry, error = %err, "invalid IP in blacklist"),
                                }
                            }
                        }
                        tracing::info!(
                            path = %path.display(),
                            ips = blacklist_ips.len(),
                            cidrs = blacklist_cidrs.len(),
                            "loaded abuse blacklist"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(path = %path.display(), error = %err, "failed to read abuse blacklist file");
                    }
                }
            } else {
                tracing::debug!(path = %path.display(), "abuse blacklist file not found, continuing with empty blacklist");
            }
        }

        IpCooldownCache {
            quarantined: DashMap::with_capacity_and_shard_amount(1024, 64),
            cooldown_duration,
            max_entries,
            blacklist_ips,
            blacklist_cidrs,
        }
    }

    /// Check whether an IP is quarantined (due to past failure) or statically blacklisted.
    pub fn is_quarantined(&self, ip: &IpAddr) -> bool {
        if self.blacklist_ips.contains(ip) {
            return true;
        }
        for cidr in &self.blacklist_cidrs {
            if cidr.contains(ip) {
                return true;
            }
        }
        if let Some(entry) = self.quarantined.get(ip) {
            if Instant::now() < *entry.value() {
                return true;
            }
            drop(entry);
            self.quarantined.remove(ip);
        }
        false
    }

    /// Quarantine an IP for `cooldown_duration` after a failed connection/probe.
    pub fn mark_failure(&self, ip: IpAddr) {
        let expiry = Instant::now() + self.cooldown_duration;
        self.quarantined.insert(ip, expiry);
        self.enforce_bound();
    }

    /// Evict all expired entries. Returns count of evicted entries.
    pub fn evict_expired(&self) -> usize {
        let now = Instant::now();
        let mut evicted = 0;
        self.quarantined.retain(|_, expiry| {
            if now >= *expiry {
                evicted += 1;
                false
            } else {
                true
            }
        });
        evicted
    }

    pub fn len(&self) -> usize {
        self.quarantined.len()
    }

    pub fn is_empty(&self) -> bool {
        self.quarantined.is_empty()
    }

    fn enforce_bound(&self) {
        if self.quarantined.len() <= self.max_entries {
            return;
        }
        let _ = self.evict_expired();
        if self.quarantined.len() <= self.max_entries {
            return;
        }
        let excess = self.quarantined.len() - self.max_entries;
        let to_remove: Vec<IpAddr> = self.quarantined.iter().take(excess).map(|e| *e.key()).collect();
        for key in to_remove {
            self.quarantined.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_ipv4_parsing_and_matching() {
        let cidr = Cidr::parse("192.168.1.0/24").unwrap();
        assert!(cidr.contains(&"192.168.1.1".parse().unwrap()));
        assert!(cidr.contains(&"192.168.1.254".parse().unwrap()));
        assert!(!cidr.contains(&"192.168.2.1".parse().unwrap()));
        assert!(!cidr.contains(&"10.0.0.1".parse().unwrap()));

        let cidr32 = Cidr::parse("1.2.3.4/32").unwrap();
        assert!(cidr32.contains(&"1.2.3.4".parse().unwrap()));
        assert!(!cidr32.contains(&"1.2.3.5".parse().unwrap()));

        let cidr0 = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(cidr0.contains(&"8.8.8.8".parse().unwrap()));
        assert!(cidr0.contains(&"1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn cidr_ipv6_parsing_and_matching() {
        let cidr = Cidr::parse("2001:db8::/32").unwrap();
        assert!(cidr.contains(&"2001:db8::1".parse().unwrap()));
        assert!(cidr.contains(&"2001:db8:ffff::1".parse().unwrap()));
        assert!(!cidr.contains(&"2001:db9::1".parse().unwrap()));
    }

    #[test]
    fn dynamic_quarantine_expiry() {
        let cache = IpCooldownCache::new(Duration::from_millis(50), 1000, None);
        let ip: IpAddr = "192.0.2.1".parse().unwrap();

        assert!(!cache.is_quarantined(&ip));
        cache.mark_failure(ip);
        assert!(cache.is_quarantined(&ip));

        std::thread::sleep(Duration::from_millis(60));
        assert!(!cache.is_quarantined(&ip));
    }
}
