//! Shared helpers for HTML-scraping web sources (DuckDuckGo, Brave, Bing).
//!
//! Each scraper differs in URL shape and result regex but agrees on:
//!   - decoding common HTML entities,
//!   - stripping HTML tags from a title chunk,
//!   - deciding whether a URL "looks like a document" worth downloading.
//!
//! Keeping these in one place stops the per-source files from drifting on
//! filter rules — researchers want the same definition of "document" no
//! matter which engine surfaces the URL.

use once_cell::sync::Lazy;
use regex::Regex;

static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
/// Matches an HTML entity: hex numeric (`&#x2019;`), decimal numeric
/// (`&#8217;`), or named (`&mdash;`).
static ENTITY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"&(#[xX][0-9A-Fa-f]+|#[0-9]+|[A-Za-z][A-Za-z0-9]*);").unwrap());

/// Map a named HTML entity to its replacement. Covers the punctuation/symbol
/// entities that actually show up in search-result titles; anything else (and
/// all accented letters) is handled by the numeric branch of [`decode_entities`].
fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "hellip" => "…",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "frac12" => "½",
        "middot" => "·",
        "bull" => "•",
        _ => return None,
    })
}

/// Decode HTML entities (named + numeric, decimal + hex) in one pass. Unknown
/// entities are left verbatim. Replaces the old hand-rolled fixed list, which
/// leaked common numeric/named entities (`&#8217;`, `&mdash;`, `&rsquo;`) into
/// titles and degraded TF-IDF/title-dedup matching.
fn decode_entities(s: &str) -> String {
    ENTITY_RE
        .replace_all(s, |caps: &regex::Captures| {
            let body = &caps[1];
            if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| caps[0].to_string())
            } else if let Some(dec) = body.strip_prefix('#') {
                dec.parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| caps[0].to_string())
            } else {
                named_entity(body)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| caps[0].to_string())
            }
        })
        .into_owned()
}

pub fn clean_title(html_chunk: &str) -> String {
    // Order matters: strip tags → decode entities → collapse whitespace → trim.
    // Entity decoding must run before the final whitespace collapse so that
    // `&nbsp;` and `&#xa0;` join the runs they create.
    let no_tags = TAG_RE.replace_all(html_chunk, "");
    let decoded = decode_entities(&no_tags);
    let collapsed = WS_RE.replace_all(&decoded, " ");
    collapsed.trim().to_string()
}

/// Heuristic: does this URL plausibly resolve to a downloadable document?
/// Used to drop landing pages, abstracts, and hub pages while keeping PDFs,
/// EPUBs, and repository download endpoints.
pub fn looks_like_doc(url: &str) -> bool {
    let u = url.to_lowercase();
    if u.contains(".pdf")
        || u.contains(".epub")
        || u.contains("/pdf/")
        || u.contains("/pdf?")
        || u.contains("download=pdf")
        || u.contains("filetype=pdf")
    {
        return true;
    }
    if u.contains("/bitstream/") || u.contains("/download/") || u.contains("getfile") {
        return true;
    }
    // arXiv abstract pages (`/abs/`) and journal article pages (`/article/` —
    // OJS/PKP, Springer, PLOS, …) are landing pages the downloader is built to
    // resolve into a PDF via `canonicalize_doc_url` + `resolve_pdf_from_html`.
    // The scrapers' own comments say they rely on that resolution, so don't drop
    // these here — only generic HTML pages (no PDF affordance) are filtered.
    if u.contains("/abs/") || u.contains("/article/") {
        return true;
    }
    let path = u.split(['?', '#']).next().unwrap_or(&u);
    !path.ends_with(".html") && !path.ends_with(".htm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_title_html_and_entities() {
        assert_eq!(
            clean_title("Attention <b>Is All</b> You Need &amp; More"),
            "Attention Is All You Need & More"
        );
        assert_eq!(clean_title("She&#39;s &nbsp; brilliant"), "She's brilliant");
    }

    #[test]
    fn decodes_numeric_and_named_entities() {
        // Numeric decimal (right single quote), em dash, ellipsis, hex.
        assert_eq!(clean_title("Bayes&#8217; Theorem"), "Bayes’ Theorem");
        assert_eq!(
            clean_title("Deep Learning &mdash; A Review&hellip;"),
            "Deep Learning — A Review…"
        );
        assert_eq!(clean_title("caf&#xe9; au lait"), "café au lait");
        // Unknown entity is left verbatim (not dropped).
        assert_eq!(clean_title("A &frobnicate; B"), "A &frobnicate; B");
    }

    #[test]
    fn doc_filter_accepts_pdfs_and_repos() {
        assert!(looks_like_doc("https://example.com/paper.pdf"));
        assert!(looks_like_doc("https://example.com/book.epub"));
        assert!(looks_like_doc("https://repo.edu/bitstream/123/paper"));
        assert!(!looks_like_doc("https://example.com/page.html"));
        assert!(!looks_like_doc("https://example.com/page.html?session=123"));
        assert!(!looks_like_doc("https://example.com/page.htm#top"));
    }

    #[test]
    fn doc_filter_accepts_resolvable_landing_pages() {
        // arXiv abstracts and journal article pages are resolved to PDFs by the
        // downloader, so they must pass the gate (previously dropped).
        assert!(looks_like_doc("https://arxiv.org/abs/1706.03762"));
        assert!(looks_like_doc(
            "https://link.springer.com/article/10.1007/s00000-000-0000-0"
        ));
        assert!(looks_like_doc(
            "https://ojs.example.org/index.php/j/article/view/10"
        ));
    }
}
