//! Defensive egress guard for the analyze layer (M2.md §2).
//!
//! This is a pure function — no DNS, no network I/O.
//! It checks scheme, IP-literal private ranges, and host-scope
//! (hostname equality with the new page's final URL host).
//!
//! DETERMINISM: pure function, no side effects.

use url::Url;

/// Decision returned by `check_probe_url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressDecision {
    /// Probe is allowed.
    Allow,
    /// Refused because scheme is not http/https.
    RefusedScheme,
    /// Refused because URL host is a private IP literal
    /// (and the page's own host is not also private/localhost).
    RefusedPrivateAddress,
    /// Refused because URL host does not match the page host.
    RefusedScope,
}

/// Check whether a probe URL is acceptable.
///
/// Rules (M2.md §2):
/// 1. Scheme must be http or https.
/// 2. If the host is an IP literal in a private/loopback/link-local/metadata range:
///    refuse UNLESS the page's own final_url host is also such a literal or "localhost".
/// 3. Host must equal the page's own final URL host, including the port
///    (explicit or scheme-default) — same-site = host+port equality, consistent
///    with the per-link rule in M2.md §5.2 item 7.
pub fn check_probe_url(probe_url: &str, page_final_url: &str) -> EgressDecision {
    let probe = match Url::parse(probe_url) {
        Ok(u) => u,
        Err(_) => return EgressDecision::RefusedScheme,
    };

    // Rule 1: scheme must be http or https
    if probe.scheme() != "http" && probe.scheme() != "https" {
        return EgressDecision::RefusedScheme;
    }

    let page_url = match Url::parse(page_final_url) {
        Ok(u) => u,
        Err(_) => return EgressDecision::RefusedScope,
    };

    let probe_host = match probe.host_str() {
        Some(h) => h,
        None => return EgressDecision::RefusedScope,
    };

    let page_host = match page_url.host_str() {
        Some(h) => h,
        None => return EgressDecision::RefusedScope,
    };

    // Rule 2: private IP literal check
    // Only applies when the probe host IS a private IP literal.
    // If the page host is also private/localhost, skip the refusal.
    let probe_is_private = is_private_ip_literal(probe_host);
    if probe_is_private {
        let page_is_private = is_private_ip_literal(page_host) || is_localhost(page_host);
        if !page_is_private {
            return EgressDecision::RefusedPrivateAddress;
        }
    }

    // Rule 3: host-scope — host+port equality (case-insensitive for hostname part).
    // port_or_known_default fills in 80/443 for http/https when port is absent,
    // so http://example.com/ and http://example.com:80/ compare equal.
    let probe_hostname = probe.host_str().unwrap_or("");
    let page_hostname = page_url.host_str().unwrap_or("");

    // Strip brackets from IPv6 literals for comparison
    let probe_h = strip_ipv6_brackets(probe_hostname);
    let page_h = strip_ipv6_brackets(page_hostname);

    if !probe_h.eq_ignore_ascii_case(page_h) {
        return EgressDecision::RefusedScope;
    }

    // Also require port equality (explicit port or scheme-default port).
    let probe_port = probe.port_or_known_default();
    let page_port = page_url.port_or_known_default();
    if probe_port != page_port {
        return EgressDecision::RefusedScope;
    }

    EgressDecision::Allow
}

/// Returns true if `host` is "localhost" (case-insensitive).
fn is_localhost(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
}

/// Strip brackets from IPv6 addresses (e.g. "[::1]" -> "::1").
fn strip_ipv6_brackets(s: &str) -> &str {
    if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Returns true if `host` is an IP literal in a private/loopback/link-local/metadata range.
///
/// Ranges checked (M2.md §2, spec §10.3):
/// - 10.0.0.0/8
/// - 172.16.0.0/12
/// - 192.168.0.0/16
/// - 169.254.0.0/16 (link-local, includes metadata 169.254.169.254)
/// - 127.0.0.0/8 (loopback)
/// - ::1 (IPv6 loopback)
/// - fd00::/8 (IPv6 ULA)
/// - ::ffff:0:0/96 (IPv4-mapped IPv6) — mapped form of the above
pub fn is_private_ip_literal(host: &str) -> bool {
    let host = strip_ipv6_brackets(host);

    // Try parsing as IPv4
    if let Ok(ipv4) = host.parse::<std::net::Ipv4Addr>() {
        return is_private_ipv4(ipv4);
    }

    // Try parsing as IPv6
    if let Ok(ipv6) = host.parse::<std::net::Ipv6Addr>() {
        return is_private_ipv6(ipv6);
    }

    // Not an IP literal at all
    false
}

fn is_private_ipv4(addr: std::net::Ipv4Addr) -> bool {
    let octets = addr.octets();
    // 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }
    // 172.16.0.0/12
    if octets[0] == 172 && (octets[1] & 0xf0) == 16 {
        return true;
    }
    // 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }
    // 169.254.0.0/16
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }
    // 127.0.0.0/8
    if octets[0] == 127 {
        return true;
    }
    false
}

fn is_private_ipv6(addr: std::net::Ipv6Addr) -> bool {
    // ::1 loopback
    if addr == std::net::Ipv6Addr::LOCALHOST {
        return true;
    }
    // fd00::/8 — ULA: first byte 0xfd
    let segs = addr.segments();
    if (segs[0] >> 8) == 0xfd {
        return true;
    }
    // ::ffff:0:0/96 — IPv4-mapped; check if the mapped address is private
    if let Some(ipv4) = addr.to_ipv4_mapped() {
        return is_private_ipv4(ipv4);
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheme_refusal() {
        assert_eq!(
            check_probe_url("file:///etc/passwd", "http://example.com/"),
            EgressDecision::RefusedScheme
        );
        assert_eq!(
            check_probe_url("ftp://example.com/", "http://example.com/"),
            EgressDecision::RefusedScheme
        );
        assert_eq!(
            check_probe_url("javascript:alert(1)", "http://example.com/"),
            EgressDecision::RefusedScheme
        );
    }

    #[test]
    fn test_http_https_allowed_same_host() {
        assert_eq!(
            check_probe_url("http://example.com/page", "http://example.com/"),
            EgressDecision::Allow
        );
        assert_eq!(
            check_probe_url("https://example.com/page", "https://example.com/"),
            EgressDecision::Allow
        );
    }

    #[test]
    fn test_scope_refusal_different_host() {
        assert_eq!(
            check_probe_url("https://evil.com/page", "https://example.com/"),
            EgressDecision::RefusedScope
        );
    }

    #[test]
    fn test_scope_refusal_subdomain_is_refused() {
        // The defensive guard uses exact hostname equality (no registrable-domain logic)
        // Subdomain of example.com is NOT example.com
        assert_eq!(
            check_probe_url("https://sub.example.com/page", "https://example.com/"),
            EgressDecision::RefusedScope
        );
    }

    #[test]
    fn test_private_ip_literal_refusal() {
        // 10.x.x.x
        assert_eq!(
            check_probe_url("http://10.0.0.1/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
        // 172.16.x.x
        assert_eq!(
            check_probe_url("http://172.16.0.1/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
        // 192.168.x.x
        assert_eq!(
            check_probe_url("http://192.168.1.1/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
        // 169.254.169.254 (metadata service)
        assert_eq!(
            check_probe_url(
                "http://169.254.169.254/latest/meta-data/",
                "http://example.com/"
            ),
            EgressDecision::RefusedPrivateAddress
        );
        // 127.x.x.x loopback
        assert_eq!(
            check_probe_url("http://127.0.0.1/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
        // ::1 IPv6 loopback
        assert_eq!(
            check_probe_url("http://[::1]/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
        // fd00::/8 ULA
        assert_eq!(
            check_probe_url("http://[fd00::1]/", "http://example.com/"),
            EgressDecision::RefusedPrivateAddress
        );
    }

    #[test]
    fn test_localhost_input_exception() {
        // When page's own host is localhost, private probes are allowed
        assert_eq!(
            check_probe_url("http://localhost:3001/foo", "http://localhost:3000/"),
            EgressDecision::RefusedScope // different host (port is not part of hostname here but localhost != localhost:3001 is actually same hostname)
        );
        // Same host exactly
        assert_eq!(
            check_probe_url("http://localhost/foo", "http://localhost/"),
            EgressDecision::Allow
        );
        // When page is 127.0.0.1, probing 10.x is still allowed (input is private)
        assert_eq!(
            check_probe_url("http://127.0.0.1/foo", "http://127.0.0.1/"),
            EgressDecision::Allow
        );
    }

    #[test]
    fn test_ipv4_private_ranges() {
        use std::net::Ipv4Addr;
        assert!(is_private_ipv4(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(10, 255, 255, 255)));
        assert!(is_private_ipv4(Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(!is_private_ipv4(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(192, 168, 0, 1)));
        assert!(is_private_ipv4(Ipv4Addr::new(169, 254, 169, 254)));
        assert!(is_private_ipv4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_private_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_private_ipv4(Ipv4Addr::new(1, 1, 1, 1)));
    }

    #[test]
    fn test_ipv6_private() {
        use std::net::Ipv6Addr;
        // ::1 loopback
        assert!(is_private_ipv6(Ipv6Addr::LOCALHOST));
        // fd00::/8
        let ula: Ipv6Addr = "fd00::1".parse().unwrap();
        assert!(is_private_ipv6(ula));
        let ula2: Ipv6Addr = "fdff::1".parse().unwrap();
        assert!(is_private_ipv6(ula2));
        // Public
        let pub_ipv6: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert!(!is_private_ipv6(pub_ipv6));
    }

    #[test]
    fn test_localhost_probe_same_host() {
        // localhost probing localhost is ok (host equality)
        assert_eq!(
            check_probe_url("http://localhost:3001/", "http://localhost:3001/"),
            EgressDecision::Allow
        );
    }
}
