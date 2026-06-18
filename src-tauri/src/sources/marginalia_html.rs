//! Zero-key web search via Marginalia Search.
//!
//! Marginalia indexes the small/old/non-commercial web — old academic
//! .edu pages, university typewritten reports, personal research blogs.
//! Surfaces things that Bing/DDG/Brave never see because their crawlers
//! prioritize commercial / popular pages.
//!
//! Endpoint: <https://search.marginalia.nu/search?query=...>. Results are
//! rendered as `<div class="card search-result">` blocks each containing
//! a single `<a>` to the destination URL.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

use super::web_common::{clean_title, looks_like_doc, read_text_capped};
use super::{Document, Source, USER_AGENT};

const ENDPOINT: &str = "https://search.marginalia.nu/search";
const MAX_PAGES: usize = 3;
const PAGE_SIZE: usize = 25;

pub struct MarginaliaHtmlSource {
    client: Arc<reqwest::Client>,
}

impl MarginaliaHtmlSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Marginalia puts each result title in an <h2> containing an <a href="...">.
// Two-stage (extract the h2 block, then find the anchor + href inside) so the
// match doesn't require the anchor to sit IMMEDIATELY after <h2> — markup like
// `<h2><span class="badge">PDF</span><a ...>` would defeat a single-stage regex
// and silently drop every Marginalia result. Mirrors the other five scrapers.
static H2_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?is)<h2\b[^>]*>(.*?)</h2>"#).unwrap());
static ANCHOR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?is)<a\s+([^>]*)>(.*?)</a>"#).unwrap());
static HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)href="(https?://[^"]+)""#).unwrap());

#[async_trait]
impl Source for MarginaliaHtmlSource {
    fn name(&self) -> &'static str {
        "marginalia"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        // Marginalia doesn't support `filetype:` or boolean `OR` — the bare words
        // `pdf`, `OR`, `epub` would just become ordinary search terms, biasing
        // results toward pages that literally contain the word "OR" and down-
        // ranking the real targets. Query plain keywords (matching Brave / Mojeek
        // / Startpage) and let `looks_like_doc` + the downloader's landing-page→PDF
        // resolution filter for documents instead.
        let q = keywords.join(" ");

        stream::unfold((0usize, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done || yielded >= limit || page >= MAX_PAGES {
                    return None;
                }
                let req = client
                    .get(ENDPOINT)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "text/html,application/xhtml+xml")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .query(&[
                        ("query", q.as_str()),
                        ("profile", "no-js"),
                        ("js", "default"),
                    ])
                    .send()
                    .await;
                let resp = match req {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if !resp.status().is_success() {
                    return Some((
                        Err(anyhow::anyhow!("marginalia http {}", resp.status())),
                        (page, yielded, true),
                    ));
                }
                let body = match read_text_capped(resp).await {
                    Ok(t) => t,
                    // read_text_capped already yields anyhow::Error.
                    Err(e) => return Some((Err(e), (page, yielded, true))),
                };

                let mut docs: Vec<Document> = Vec::new();
                let mut count = 0usize;
                // Raw result rows, counted separately from filtered `docs` so we
                // only stop on a genuinely empty page.
                let mut raw = 0usize;
                for h2 in H2_RE.captures_iter(&body) {
                    let inner = h2.get(1).map(|m| m.as_str()).unwrap_or("");
                    let Some(anchor) = ANCHOR_RE.captures(inner) else {
                        continue;
                    };
                    raw += 1;
                    if yielded + count >= limit {
                        break;
                    }
                    let attrs = anchor.get(1).map(|m| m.as_str()).unwrap_or("");
                    let title_html = anchor.get(2).map(|m| m.as_str()).unwrap_or("");
                    let url = HREF_RE
                        .captures(attrs)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default();
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
                        source: "marginalia".to_string(),
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
                // Marginalia has no stable offset/page parameter on this endpoint,
                // so a second request would just refetch the identical first page
                // (every doc then deduped away — a wasted round-trip). Stop after
                // one page. PAGE_SIZE is retained only as documentation.
                let _ = PAGE_SIZE;
                Some((Ok(docs), (page + 1, yielded + count, true)))
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

    // Helper mirroring the production loop: pull (url, title) out of an <h2>.
    fn extract(html: &str) -> Option<(String, String)> {
        let h2 = H2_RE.captures(html)?;
        let inner = h2.get(1).map(|m| m.as_str()).unwrap_or("");
        let anchor = ANCHOR_RE.captures(inner)?;
        let attrs = anchor.get(1).map(|m| m.as_str()).unwrap_or("");
        let title_html = anchor.get(2).map(|m| m.as_str()).unwrap_or("");
        let url = HREF_RE
            .captures(attrs)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())?;
        Some((url, clean_title(title_html)))
    }

    #[test]
    fn extracts_marginalia_anchor() {
        let html = r#"
            <div class="card search-result">
                <h2><a href="https://stanford.edu/old/papers/network.pdf">
                    A 1996 paper on small-world networks
                </a></h2>
                <p>...</p>
            </div>
        "#;
        let (url, title) = extract(html).expect("matches");
        assert_eq!(url, "https://stanford.edu/old/papers/network.pdf");
        assert_eq!(title, "A 1996 paper on small-world networks");
    }

    #[test]
    fn extracts_anchor_not_immediately_after_h2() {
        // The single-stage regex this replaced required the <a> to sit right
        // after <h2>; markup with an interleaved badge silently dropped the row.
        let html = r#"
            <h2><span class="badge">PDF</span><a href="https://mit.edu/report.pdf">
                Typewritten lab report
            </a></h2>
        "#;
        let (url, title) = extract(html).expect("matches despite leading badge");
        assert_eq!(url, "https://mit.edu/report.pdf");
        assert_eq!(title, "Typewritten lab report");
    }
}
