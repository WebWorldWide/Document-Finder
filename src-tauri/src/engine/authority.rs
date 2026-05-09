//! Domain authority multipliers for ranked search results.
//!
//! Boosts results from authoritative sources so that when two documents
//! score similarly on relevance, the one from a peer-reviewed publisher
//! or government archive wins. The list is intentionally short and
//! conservative — over-boosting `.edu` will drown out a better-matching
//! preprint server. All multipliers are between 1.0 and 1.4.
//!
//! Add new entries at PR review time; cite a reason in the commit message
//! ("PLOS open-access journals" rather than "looks legit").

use url::Url;

/// (host suffix, multiplier). Matched right-to-left against the URL host so
/// `mit.edu` matches `arxiv.mit.edu`. Order matters only for ties; first
/// match wins.
const PUBLISHERS: &[(&str, f32)] = &[
    // Top-tier publishers and academic societies.
    ("nature.com", 1.30),
    ("science.org", 1.30),
    ("cell.com", 1.30),
    ("nejm.org", 1.30),
    ("ieeexplore.ieee.org", 1.25),
    ("ieee.org", 1.25),
    ("acm.org", 1.25),
    ("springer.com", 1.20),
    ("sciencedirect.com", 1.20),
    ("wiley.com", 1.20),
    ("oup.com", 1.20),
    ("cambridge.org", 1.20),
    ("jstor.org", 1.20),
    ("plos.org", 1.20),
    ("frontiersin.org", 1.15),
    ("mdpi.com", 1.10),
    ("biorxiv.org", 1.15),
    ("medrxiv.org", 1.15),
    // Established scholarly indexes already returning canonical metadata.
    ("arxiv.org", 1.20),
    ("semanticscholar.org", 1.15),
    ("openalex.org", 1.15),
    ("doaj.org", 1.10),
    ("archive.org", 1.10),
    ("gutenberg.org", 1.10),
];

/// Returns a multiplicative score boost in `[1.0, 1.40]` for a result URL.
///
/// `.gov` and `.edu` get a flat 1.40 / 1.30 — for serious research these
/// are usually the highest-signal hosts when one appears.
pub fn authority_multiplier(url: &str) -> f32 {
    let Ok(parsed) = Url::parse(url) else {
        return 1.0;
    };
    let Some(host) = parsed.host_str() else {
        return 1.0;
    };
    let host = host.to_lowercase();

    // TLD-level boosts apply broadly.
    if host.ends_with(".gov") {
        return 1.40;
    }
    if host.ends_with(".edu") {
        return 1.30;
    }

    // Suffix match against the publisher table.
    for (suffix, mult) in PUBLISHERS {
        if host == *suffix || host.ends_with(&format!(".{}", suffix)) {
            return *mult;
        }
    }

    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_gov_gets_top_boost() {
        assert!((authority_multiplier("https://www.loc.gov/items/x.pdf") - 1.40).abs() < 1e-6);
    }

    #[test]
    fn dot_edu_under_gov() {
        let edu = authority_multiplier("https://stanford.edu/papers/y.pdf");
        let gov = authority_multiplier("https://nasa.gov/x.pdf");
        assert!(edu < gov && edu > 1.0);
    }

    #[test]
    fn publisher_subdomain_match() {
        assert!(authority_multiplier("https://link.springer.com/article/10.1007/x.pdf") > 1.0);
    }

    #[test]
    fn unknown_domain_neutral() {
        assert_eq!(
            authority_multiplier("https://random-blog.example.com/post.pdf"),
            1.0
        );
    }

    #[test]
    fn malformed_url_neutral() {
        assert_eq!(authority_multiplier("not a url at all"), 1.0);
    }
}
