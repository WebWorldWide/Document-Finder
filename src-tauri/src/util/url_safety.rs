//! SSRF-mitigation helpers for user-supplied and remotely-fetched URLs.
//!
//! Validates that a URL is safe to fetch: correct scheme, no embedded
//! credentials, and hostname resolves to a public IP address (not RFC1918,
//! loopback, link-local, or multicast). Designed as a best-effort defence
//! for a desktop app — not a substitute for network-level controls.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

/// Validate `raw` is an **https** URL pointing at a public host.
///
/// Checks (in order):
/// 1. Parses as a valid URL.
/// 2. Scheme is exactly `https`.
/// 3. No embedded credentials (user/password in the authority).
/// 4. Hostname resolves to at least one IP, all of which are public.
///
/// Returns the parsed `Url` on success, or a human-readable error on failure.
pub async fn validate_url(raw: &str) -> Result<Url, String> {
    validate_url_inner(raw, &["https"]).await
}

/// Validate `raw` is a document URL safe to download.
///
/// Same envelope as [`validate_url`] (no credentials, public-IP-only host) but
/// accepts `http` **or** `https`, because open-access publishers and indexers
/// sometimes hand out plain `http://` PDF links. Still blocks private,
/// loopback, link-local (incl. the cloud-metadata `169.254.169.254`), and
/// IPv4-mapped-private targets so a malicious search result can't turn the app
/// into an SSRF proxy.
pub async fn validate_download_url(raw: &str) -> Result<Url, String> {
    validate_url_inner(raw, &["http", "https"]).await
}

async fn validate_url_inner(raw: &str, allowed_schemes: &[&str]) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;

    if !allowed_schemes.contains(&url.scheme()) {
        return Err(format!(
            "Rejected: scheme '{}' is not allowed (expected {})",
            url.scheme(),
            allowed_schemes.join(" or ")
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err("Rejected: URL must not contain credentials".into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "Rejected: URL has no host".to_string())?;

    // Resolve against the URL's effective port so default-port hosts still
    // resolve; the port itself doesn't affect the privacy decision.
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("DNS resolution failed for '{host}': {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("Rejected: '{host}' did not resolve to any address"));
    }

    for addr in &addrs {
        let ip = addr.ip();
        if is_private_ip(&ip) {
            return Err(format!(
                "Rejected: '{host}' resolves to a private/reserved IP ({ip})"
            ));
        }
    }

    Ok(url)
}

/// True if `ip` is loopback, private, link-local, multicast, unspecified, or
/// otherwise non-public. Shared with the redirect policy in `sources::mod` so
/// redirects to internal IP literals are blocked too.
pub(crate) fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(v4: &Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        // CGNAT: 100.64.0.0/10
        || (u32::from(*v4) & 0xFFC0_0000 == 0x6440_0000)
}

fn is_private_v6(v6: &Ipv6Addr) -> bool {
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // ULA fc00::/7
        || (v6.segments()[0] & 0xFE00 == 0xFC00)
        // Link-local fe80::/10
        || (v6.segments()[0] & 0xFFC0 == 0xFE80)
        // IPv4-mapped (::ffff:a.b.c.d) — judge by the embedded IPv4 so
        // ::ffff:127.0.0.1 etc. can't slip past the v6 checks.
        || v6.to_ipv4_mapped().is_some_and(|v4| is_private_v4(&v4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_private() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn rfc1918_is_private() {
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn unspecified_is_private() {
        assert!(is_private_ip(&"0.0.0.0".parse().unwrap()));
        assert!(is_private_ip(&"::".parse().unwrap()));
    }

    #[test]
    fn link_local_and_metadata_is_private() {
        // 169.254.169.254 (cloud metadata) lives in the link-local block.
        assert!(is_private_ip(&"169.254.169.254".parse().unwrap()));
        assert!(is_private_ip(&"fe80::1".parse().unwrap()));
    }

    #[test]
    fn ipv4_mapped_is_judged_by_embedded_v4() {
        assert!(is_private_ip(&"::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse().unwrap()));
        assert!(!is_private_ip(&"::ffff:1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn public_ip_is_not_private() {
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"2606:4700:4700::1111".parse().unwrap()));
    }
}
