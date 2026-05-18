//! SSRF-mitigation helpers for user-supplied and remotely-fetched URLs.
//!
//! Validates that a URL is safe to fetch: correct scheme, no embedded
//! credentials, and hostname resolves to a public IP address (not RFC1918,
//! loopback, link-local, or multicast). Designed as a best-effort defence
//! for a desktop app — not a substitute for network-level controls.

use std::net::IpAddr;
use url::Url;

/// Validate `raw` is an https URL pointing at a public host.
///
/// Checks (in order):
/// 1. Parses as a valid URL.
/// 2. Scheme is exactly `https`.
/// 3. No embedded credentials (user/password in the authority).
/// 4. Hostname resolves to at least one IP, all of which are public.
///
/// Returns the parsed `Url` on success, or a human-readable error on failure.
pub async fn validate_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;

    if url.scheme() != "https" {
        return Err(format!(
            "Rejected: scheme '{}' is not https",
            url.scheme()
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err("Rejected: URL must not contain credentials".into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "Rejected: URL has no host".to_string())?;

    let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:443"))
        .await
        .map_err(|e| format!("DNS resolution failed for '{host}': {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "Rejected: '{host}' did not resolve to any address"
        ));
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

/// Synchronous variant for contexts where async is not available.
/// Does NOT perform DNS resolution — only validates scheme, credentials,
/// and parses the URL. Call the async `validate_url` for full SSRF protection.
pub fn validate_url_sync(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;

    if url.scheme() != "https" {
        return Err(format!(
            "Rejected: scheme '{}' is not https",
            url.scheme()
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err("Rejected: URL must not contain credentials".into());
    }

    url.host_str()
        .ok_or_else(|| "Rejected: URL has no host".to_string())?;

    Ok(url)
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                // CGNAT: 100.64.0.0/10
                || (u32::from(*v4) & 0xFFC0_0000 == 0x6440_0000)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_multicast()
                // ULA fc00::/7
                || (v6.segments()[0] & 0xFE00 == 0xFC00)
                // Link-local fe80::/10
                || (v6.segments()[0] & 0xFFC0 == 0xFE80)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_http_scheme() {
        assert!(validate_url_sync("http://example.com/path").is_err());
    }

    #[test]
    fn rejects_file_scheme() {
        assert!(validate_url_sync("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_credentials_in_url() {
        assert!(validate_url_sync("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn accepts_clean_https() {
        assert!(validate_url_sync("https://searx.space/data/instances.json").is_ok());
    }

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
    fn public_ip_is_not_private() {
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
    }
}
