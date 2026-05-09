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
// containing the title. The class set has shifted occasionally; this regex
// tolerates additional classes around `result-header`.
static RESULT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?is)<a\s+[^>]*class="[^"]*\bresult-header\b[^"]*"\s+[^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap()
});

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
        let q = format!("{} filetype:pdf OR filetype:epub", keywords.join(" "));

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
                for cap in RESULT_RE.captures_iter(&body) {
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
                        source: "brave".to_string(),
                        authors: Vec::new(),
                        year: None,
                        abstract_: None,
                        identifier: None,
                    });
                    count += 1;
                }

                if docs.is_empty() {
                    return None;
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

    #[test]
    fn extracts_brave_result_anchor() {
        let html = r#"
            <div>
              <a class="result-header svelte-xyz" href="https://example.edu/paper.pdf" tabindex="0">
                Quantum <em>Entanglement</em> Survey
              </a>
            </div>
        "#;
        let cap = RESULT_RE.captures(html).expect("matches");
        assert_eq!(&cap[1], "https://example.edu/paper.pdf");
        assert_eq!(
            clean_title(cap.get(2).unwrap().as_str()),
            "Quantum Entanglement Survey"
        );
    }
}
