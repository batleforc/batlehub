//! Parsing for the `trusted_proxies` lists (RFC 0001 §4.5).
//!
//! Two config keys feed the same decision:
//!
//! - `[server].trusted_proxies` — the current, server-level list. Governs the
//!   `Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` / `X-Forwarded-For`
//!   headers alike.
//! - `[ip_blocking].trusted_proxies` — the deprecated alias, kept working so no
//!   existing config breaks. Used only when `[server]` declares none.
//!
//! Entries are CIDR ranges (`10.42.0.0/16`) or bare addresses, which are widened
//! to a `/32` (IPv4) or `/128` (IPv6) so every value that was valid under the old
//! exact-match rule still matches exactly what it used to.
//!
//! `Option` is load-bearing throughout: **absent** (`None`) and **configured to
//! trust nobody** (`Some(vec![])`) are different states, and the callers of
//! [`resolve_trusted_proxies`] treat them differently.

use anyhow::{bail, Result};
use ipnet::IpNet;
use std::net::IpAddr;

/// Parse one `trusted_proxies` entry: either a CIDR block or a bare address.
///
/// A bare address becomes a single-host prefix, so `"10.0.0.1"` matches exactly
/// `10.0.0.1` and nothing else.
pub fn parse_entry(entry: &str) -> Result<IpNet> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        bail!("trusted_proxies entry is empty");
    }
    if let Ok(net) = trimmed.parse::<IpNet>() {
        return Ok(net);
    }
    match trimmed.parse::<IpAddr>() {
        Ok(addr) => Ok(IpNet::from(addr)),
        Err(_) => bail!(
            "invalid trusted_proxies entry '{trimmed}': expected an IP address \
             (10.0.0.1) or a CIDR range (10.42.0.0/16)"
        ),
    }
}

/// Parse a whole list, failing on the first malformed entry.
///
/// Validating rather than silently skipping matters: an unparseable entry used
/// to mean "this proxy is not trusted", which fails closed for client IPs but is
/// invisible to the operator who typo'd it.
pub fn parse_list(entries: &[String]) -> Result<Vec<IpNet>> {
    entries.iter().map(|e| parse_entry(e)).collect()
}

/// Pick the effective list from the two config keys, newest key first.
///
/// Returns `None` when neither key is set — the legacy state, which callers map
/// to today's behaviour rather than to "trust nobody".
pub fn resolve_trusted_proxies<'a>(
    server: Option<&'a [String]>,
    ip_blocking: Option<&'a [String]>,
) -> Option<&'a [String]> {
    server.or(ip_blocking)
}

/// Whether `peer` falls inside any of `nets`.
pub fn contains(nets: &[IpNet], peer: IpAddr) -> bool {
    nets.iter().any(|n| n.contains(&peer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_ipv4_becomes_a_host_prefix() {
        let net = parse_entry("10.0.0.1").unwrap();
        assert_eq!(net.prefix_len(), 32);
        assert!(net.contains(&"10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(!net.contains(&"10.0.0.2".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn bare_ipv6_becomes_a_host_prefix() {
        let net = parse_entry("2001:db8::1").unwrap();
        assert_eq!(net.prefix_len(), 128);
        assert!(net.contains(&"2001:db8::1".parse::<IpAddr>().unwrap()));
        assert!(!net.contains(&"2001:db8::2".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn cidr_matches_the_whole_range() {
        let net = parse_entry("10.42.0.0/16").unwrap();
        assert!(net.contains(&"10.42.7.9".parse::<IpAddr>().unwrap()));
        assert!(!net.contains(&"10.43.0.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert!(parse_entry("  10.0.0.1  ").is_ok());
    }

    #[test]
    fn malformed_entries_are_rejected() {
        for bad in ["", "   ", "not-an-ip", "10.0.0.1/33", "10.0.0.0/", "::/999"] {
            assert!(parse_entry(bad).is_err(), "'{bad}' should not parse");
        }
    }

    #[test]
    fn parse_list_reports_the_first_bad_entry() {
        let entries = vec!["10.0.0.1".to_owned(), "nope".to_owned()];
        let err = parse_list(&entries).unwrap_err().to_string();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn contains_spans_mixed_families() {
        let nets = parse_list(&["10.42.0.0/16".to_owned(), "2001:db8::/32".to_owned()]).unwrap();
        assert!(contains(&nets, "10.42.0.1".parse().unwrap()));
        assert!(contains(&nets, "2001:db8::5".parse().unwrap()));
        assert!(!contains(&nets, "192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn server_key_wins_over_the_deprecated_alias() {
        let server = vec!["10.0.0.1".to_owned()];
        let alias = vec!["192.0.2.1".to_owned()];
        assert_eq!(
            resolve_trusted_proxies(Some(&server), Some(&alias)),
            Some(server.as_slice())
        );
    }

    #[test]
    fn alias_is_used_when_the_server_key_is_absent() {
        let alias = vec!["192.0.2.1".to_owned()];
        assert_eq!(
            resolve_trusted_proxies(None, Some(&alias)),
            Some(alias.as_slice())
        );
    }

    #[test]
    fn an_empty_server_list_still_shadows_the_alias() {
        // `[server] trusted_proxies = []` means "trust nobody" — it must not
        // fall through to the deprecated key.
        let server: Vec<String> = Vec::new();
        let alias = vec!["192.0.2.1".to_owned()];
        assert_eq!(
            resolve_trusted_proxies(Some(&server), Some(&alias)),
            Some(server.as_slice())
        );
    }

    #[test]
    fn neither_key_resolves_to_none() {
        assert_eq!(resolve_trusted_proxies(None, None), None);
    }
}
