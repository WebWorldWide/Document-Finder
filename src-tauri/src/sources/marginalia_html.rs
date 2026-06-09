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

use super::web_common::{clean_title, looks_like_doc};
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

// Marginalia wraps each result in a `search-result` div; the title link
// is the first <a href="..."> inside an <h2>.
static RESULT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<h2[^>]*>\s*<a\s+[^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>\s*</h2>"#)
        .unwrap()
});

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
                let body = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };

                let mut docs: Vec<Document> = Vec::new();
                let mut count = 0usize;
                // Raw result rows, counted separately from filtered `docs` so we
                // only stop on a genuinely empty page.
                let mut raw = 0usize;
                for cap in RESULT_RE.captures_iter(&body) {
                    raw += 1;
                    if yielded + count >= limit {
                        break;
                    }
                    let url = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
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
                // Marginalia only paginates via re-querying; for now we
                // bail after one page (most queries return < PAGE_SIZE).
                let _ = PAGE_SIZE;
                Some((Ok(docs), (page + 1, yielded + count, page >= 1)))
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
        let cap = RESULT_RE.captures(html).expect("matches");
        assert_eq!(&cap[1], "https://stanford.edu/old/papers/network.pdf");
        assert_eq!(
            clean_title(cap.get(2).unwrap().as_str()),
            "A 1996 paper on small-world networks"
        );
    }
}
