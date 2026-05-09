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

pub fn clean_title(html_chunk: &str) -> String {
    // Order matters: strip tags → decode entities → collapse whitespace → trim.
    // Entity decoding must run before the final whitespace collapse so that
    // `&nbsp;` and `&#xa0;` join the runs they create.
    let no_tags = TAG_RE.replace_all(html_chunk, "");
    let decoded = no_tags
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&nbsp;", " ");
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
    !u.ends_with(".html")
        && !u.ends_with(".htm")
        && !u.contains("/abs/")
        && !u.contains("/article/")
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
        assert_eq!(
            clean_title("She&#39;s &nbsp; brilliant"),
            "She's brilliant"
        );
    }

    #[test]
    fn doc_filter_accepts_pdfs_and_repos() {
        assert!(looks_like_doc("https://example.com/paper.pdf"));
        assert!(looks_like_doc("https://example.com/book.epub"));
        assert!(looks_like_doc("https://repo.edu/bitstream/123/paper"));
        assert!(!looks_like_doc("https://example.com/page.html"));
        assert!(!looks_like_doc("https://arxiv.org/abs/1706.03762"));
    }
}
