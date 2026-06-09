//! Zero-key web search via Brave Search's HTML results page.
//!
//! Brave's UI has a stable per-result anchor with a `result-header` class
//! that wraps the canonical destination URL. Scraping this is fragile to
//! markup changes but requires no API key and gives different coverage
//! from DuckDuckGo (Brave has its own crawler instead of mirroring Google).

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

use super::web_common::{clean_title, looks_like_doc};
use super::{Document, Source, USER_AGENT};

const ENDPOINT: &str = "https://search.brave.com/search";
const MAX_PAGES: usize = 4;
const PAGE_SIZE: usize = 20;

pub struct BraveHtmlSource {
    client: Arc<reqwest::Client>,
}

impl BraveHtmlSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Brave wraps each result in a `<a class="result-header" href="...">` anchor
// containing the title. We match in TWO stages — capture each anchor's attribute
// blob + inner HTML, then test for the class and extract href SEPARATELY — so the
// attribute ORDER doesn't matter. A single regex requiring class-before-href
// silently matches nothing whenever the engine emits `href` first (HTML
// attribute order is not guaranteed), making the whole engine contribute zero
// without ever tripping the circuit breaker.
static ANCHOR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?is)<a\s+([^>]*)>(.*?)</a>"#).unwrap());
static HEADER_CLASS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)class="[^"]*\bresult-header\b[^"]*""#).unwrap());
static HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)href="(https?://[^"]+)""#).unwrap());

#[async_trait]
impl Source for BraveHtmlSource {
    fn name(&self) -> &'static str {
        "brave"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        // Brave doesn't reliably honor Google/Bing-style `filetype:` operators —
        // the literal tokens leak in as search terms and skew/empty results.
        // Query plain keywords; looks_like_doc + the downloader's landing-page→PDF
        // resolution filter for documents instead.
        let q = keywords.join(" ");

        stream::unfold((0usize, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done || yielded >= limit || page >= MAX_PAGES {
                    return None;
                }
                let offset = (page * PAGE_SIZE).to_string();
                let req = client
                    .get(ENDPOINT)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "text/html,application/xhtml+xml")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .query(&[
                        ("q", q.as_str()),
                        ("source", "web"),
                        ("offset", offset.as_str()),
                    ])
                    .send()
                    .await;
                let resp = match req {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if !resp.status().is_success() {
                    return Some((
                        Err(anyhow::anyhow!("brave http {}", resp.status())),
                        (page, yielded, true),
                    ));
                }
                let body = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };

                let mut docs: Vec<Document> = Vec::new();
                let mut count = 0usize;
                // Count RAW result rows separately from filtered `docs` so we only
                // stop paginating on a genuinely empty page, not a page whose
                // results all failed `looks_like_doc`.
                let mut raw = 0usize;
                for cap in ANCHOR_RE.captures_iter(&body) {
                    let attrs = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    if !HEADER_CLASS_RE.is_match(attrs) {
                        continue; // not a result anchor at all
                    }
                    raw += 1;
                    if yielded + count >= limit {
                        break;
                    }
                    let url = HREF_RE
                        .captures(attrs)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let title_html = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                    if url.is_empty() || !looks_like_doc(&url) {
                        continue;
                    }
                    let title = clean_title(title_html);
                    if title.is_empty() {
                        continue;
                    }
                    docs.push(Document {
                        title,
                        url,
                        source: "brave".to_string(),
                        authors: Vec::new(),
                        year: None,
                        abstract_: None,
                        identifier: None,
                    });
                    count += 1;
                }

                if raw == 0 {
                    return None; // zero raw results on this page → end of results
                }
                Some((Ok(docs), (page + 1, yielded + count, false)))
            }
        })
        .flat_map(|res: anyhow::Result<Vec<Document>>| match res {
            Ok(docs) => stream::iter(docs.into_iter().map(Ok).collect::<Vec<_>>()).boxed(),
            Err(e) => stream::iter(vec![Err(e)]).boxed(),
        })
        .take(limit)
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract (url, title) pairs exactly as the search loop does.
    fn extract(html: &str) -> Vec<(String, String)> {
        ANCHOR_RE
            .captures_iter(html)
            .filter_map(|cap| {
                let attrs = cap.get(1)?.as_str();
                if !HEADER_CLASS_RE.is_match(attrs) {
                    return None;
                }
                let url = HREF_RE.captures(attrs)?.get(1)?.as_str().to_string();
                let title = clean_title(cap.get(2)?.as_str());
                Some((url, title))
            })
            .collect()
    }

    #[test]
    fn extracts_brave_result_anchor_either_attribute_order() {
        // HTML attribute order is not guaranteed — both must parse.
        let class_first = r#"<a class="result-header svelte-xyz" href="https://example.edu/paper.pdf" tabindex="0">Quantum <em>Entanglement</em> Survey</a>"#;
        let href_first = r#"<a href="https://example.edu/paper.pdf" class="result-header svelte-xyz" tabindex="0">Quantum <em>Entanglement</em> Survey</a>"#;
        for html in [class_first, href_first] {
            let got = extract(html);
            assert_eq!(got.len(), 1, "failed to match: {html}");
            assert_eq!(got[0].0, "https://example.edu/paper.pdf");
            assert_eq!(got[0].1, "Quantum Entanglement Survey");
        }
    }
}
