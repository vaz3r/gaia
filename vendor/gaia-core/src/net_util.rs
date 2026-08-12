//! Network-address classification helpers shared across the engine.
//!
//! Relocated from `irontide-session::rate_limiter` at M244a so `ip_filter` (now in
//! `irontide-session-types`) and the session callers (`url_guard`, `torrent_dispatch`)
//! share one definition without a torrent→session back-edge.
use std::net::IpAddr;

/// Check if an IP address is on a local/private network.
///
/// IPv4: loopback, private (RFC 1918), link-local (169.254.0.0/16), unspecified
/// (`0.0.0.0`).
/// IPv6: loopback (`::1`), link-local (`fe80::/10`), unique-local / ULA
/// (`fc00::/7`), unspecified (`::`).
///
/// IPv4-mapped IPv6 addresses (`::ffff:x.x.x.x`) are normalised to IPv4 first
/// so the v4 RFC 1918 / loopback / link-local checks catch them — without this
/// the IPv6 branch only matches literal `::1`, which leaves `::ffff:127.0.0.1`,
/// `::ffff:192.168.1.1`, etc. unprotected against SSRF.
///
/// The unspecified-address checks (`0.0.0.0`, `::`) close another SSRF gap:
/// on Linux, connecting to `0.0.0.0:p` resolves to `127.0.0.1:p`, so a URL
/// like `http://0.0.0.0/` is a loopback request that the underlying RFC 1918
/// / loopback predicates miss (`is_loopback()` only matches `127.0.0.0/8`).
pub fn is_local_network(addr: IpAddr) -> bool {
    // Normalize IPv4-mapped IPv6 to IPv4 so the v4 predicates apply uniformly.
    let addr = match addr {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        IpAddr::V4(_) => addr,
    };
    match addr {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            if ip.is_loopback() || ip.is_unspecified() {
                return true;
            }
            let octets = ip.octets();
            // fe80::/10 — link-local
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 {
                return true;
            }
            // fc00::/7 — unique-local (ULA)
            if (octets[0] & 0xfe) == 0xfc {
                return true;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_network_detection() {
        assert!(is_local_network("127.0.0.1".parse().unwrap()));
        assert!(is_local_network("192.168.1.1".parse().unwrap()));
        assert!(is_local_network("10.0.0.1".parse().unwrap()));
        assert!(is_local_network("172.16.0.1".parse().unwrap()));
        assert!(is_local_network("169.254.1.1".parse().unwrap()));
        assert!(is_local_network("::1".parse().unwrap()));
        assert!(!is_local_network("8.8.8.8".parse().unwrap()));
        assert!(!is_local_network("1.2.3.4".parse().unwrap()));
    }

    #[test]
    fn ipv6_local_network_detection() {
        // Loopback
        assert!(is_local_network("::1".parse().unwrap()));
        // Link-local (fe80::/10)
        assert!(is_local_network("fe80::1".parse().unwrap()));
        assert!(is_local_network("fe80::abcd:1234".parse().unwrap()));
        // Unique-local / ULA (fc00::/7)
        assert!(is_local_network("fc00::1".parse().unwrap()));
        assert!(is_local_network("fd00::1".parse().unwrap()));
        assert!(is_local_network("fd12:3456:789a::1".parse().unwrap()));
        // Global unicast — not local
        assert!(!is_local_network("2001:db8::1".parse().unwrap()));
        assert!(!is_local_network(
            "2607:f8b0:4004:800::200e".parse().unwrap()
        ));
    }

    #[test]
    fn unspecified_v4_is_local() {
        // 0.0.0.0 on Linux connects to 127.0.0.1 — must be treated as local.
        assert!(is_local_network("0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn unspecified_v6_is_local() {
        // :: (all-zeros IPv6) is the v6 equivalent — same SSRF concern.
        assert!(is_local_network("::".parse().unwrap()));
    }

    #[test]
    fn ipv4_mapped_v6_loopback_is_local() {
        // ::ffff:127.0.0.1 is loopback expressed as IPv4-mapped IPv6.
        // Ipv6Addr::is_loopback() only matches literal ::1, so without
        // to_ipv4_mapped() normalisation this would slip past the SSRF check.
        assert!(is_local_network("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_local_network("::ffff:7f00:1".parse().unwrap()));
    }

    #[test]
    fn ipv4_mapped_v6_private_is_local() {
        // Same gap for RFC 1918 ranges expressed as IPv4-mapped IPv6.
        assert!(is_local_network("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_local_network("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_local_network("::ffff:172.16.0.1".parse().unwrap()));
        // Public IPv4 mapped into IPv6 is still public.
        assert!(!is_local_network("::ffff:8.8.8.8".parse().unwrap()));
    }
}
