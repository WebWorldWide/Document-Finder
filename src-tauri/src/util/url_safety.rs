//! SSRF-mitigation helpers for user-supplied and remotely-fetched URLs.
//!
//! Validates that a URL is safe to fetch: correct scheme, no embedded
//! credentials, and hostname resolves to a public IP address (not RFC1918,
//! loopback, link-local, or multicast). Designed as a best-effort defence
//! for a desktop app — not a substitute for network-level controls.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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

/// A reqwest DNS resolver that drops private/reserved IPs at resolution time.
///
/// This closes two gaps the one-shot `validate_*` lookups and the synchronous
/// redirect policy cannot:
/// 1. **DNS-rebinding TOCTOU** — `validate_download_url` resolves a host once,
///    then reqwest resolves it again at connect time. A malicious host (download
///    URLs come from untrusted search results) can answer the first lookup with
///    a public IP and the second with `127.0.0.1` / `169.254.169.254`. Resolving
///    inside reqwest's own connection path means there is only one lookup.
/// 2. **Redirect to a private *hostname*** — the redirect policy can only inspect
///    IP literals; a 302 to a hostname that resolves private slipped through.
///    This resolver runs for every hop, including redirects.
///
/// reqwest only consults a custom resolver for **hostname** hosts; IP-literal
/// hosts (e.g. the in-process SearXNG at `127.0.0.1:<port>`) bypass DNS entirely
/// and remain governed by `validate_*` + the redirect policy — so installing
/// this globally does not break the loopback local-SearXNG fallback.
#[derive(Debug, Default, Clone)]
pub struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            // Port 0: reqwest substitutes the real destination port; only the
            // resolved IPs matter for the privacy decision.
            let resolved = tokio::net::lookup_host(format!("{host}:0")).await?;
            let public: Vec<SocketAddr> = resolved.filter(|a| !is_private_ip(&a.ip())).collect();
            if public.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    format!("blocked: '{host}' resolves only to private/reserved addresses").into();
                return Err(err);
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
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
    let bits = u32::from(*v4);
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        // CGNAT: 100.64.0.0/10
        || (bits & 0xFFC0_0000 == 0x6440_0000)
        // Benchmarking 198.18.0.0/15 (RFC 2544)
        || (bits & 0xFFFE_0000 == 0xC612_0000)
        // IETF protocol assignments 192.0.0.0/24 (RFC 6890; incl. 192.0.0.171)
        || (bits & 0xFFFF_FF00 == 0xC000_0000)
        // 6to4 anycast relay 192.88.99.0/24
        || (bits & 0xFFFF_FF00 == 0xC058_6300)
}

fn is_private_v6(v6: &Ipv6Addr) -> bool {
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // ULA fc00::/7
        || (v6.segments()[0] & 0xFE00 == 0xFC00)
        // Link-local fe80::/10
        || (v6.segments()[0] & 0xFFC0 == 0xFE80)
        // NAT64 well-known prefix 64:ff9b::/96. On IPv6-only / NAT64 networks
        // (common on mobile carriers) the OS synthesizes these to reach PUBLIC
        // IPv4 hosts — that's the normal path to the internet, so blocking the
        // whole prefix would make legitimate sources unreachable. Judge it by the
        // EMBEDDED IPv4 (low 32 bits, RFC 6052 /96 layout) instead: block only
        // when that resolves to a private address. `to_ipv4()` can't extract it
        // here (the high 96 bits are non-zero), so pull it from segments [6]/[7].
        || {
            let s = v6.segments();
            s[0..6] == [0x0064, 0xFF9B, 0, 0, 0, 0]
                && is_private_v4(&Ipv4Addr::new(
                    (s[6] >> 8) as u8,
                    (s[6] & 0xff) as u8,
                    (s[7] >> 8) as u8,
                    (s[7] & 0xff) as u8,
                ))
        }
        // Embedded IPv4 — judge by the embedded address so neither the modern
        // ::ffff:a.b.c.d (IPv4-mapped) NOR the deprecated-but-parseable
        // ::a.b.c.d (IPv4-compatible) form can smuggle 127.0.0.1 /
        // 169.254.169.254 etc. past the v6 checks. `to_ipv4` covers both forms;
        // `to_ipv4_mapped` would miss the compatible one.
        || v6.to_ipv4().is_some_and(|v4| is_private_v4(&v4))
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
    fn ipv4_compatible_is_judged_by_embedded_v4() {
        // Deprecated ::a.b.c.d form (high 96 bits zero, not 0xffff) must not
        // smuggle loopback / metadata past the v6 classifier.
        assert!(is_private_ip(&"::127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn nat64_is_judged_by_embedded_v4() {
        // 64:ff9b::/96 to a PUBLIC IPv4 is the normal path on NAT64 networks —
        // must be allowed; only a private-embedded address is blocked.
        assert!(!is_private_ip(&"64:ff9b::1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"64:ff9b::8.8.8.8".parse().unwrap()));
        assert!(is_private_ip(&"64:ff9b::127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"64:ff9b::10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"64:ff9b::169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn reserved_v4_ranges_are_private() {
        assert!(is_private_ip(&"198.18.0.1".parse().unwrap())); // benchmarking
        assert!(is_private_ip(&"192.0.0.171".parse().unwrap())); // IETF protocol
        assert!(is_private_ip(&"192.88.99.1".parse().unwrap())); // 6to4 relay
    }

    #[test]
    fn public_ip_is_not_private() {
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"2606:4700:4700::1111".parse().unwrap()));
    }
}
