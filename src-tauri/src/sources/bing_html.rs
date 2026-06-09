//! Zero-key web search via Bing's HTML results page.
//!
//! Bing renders each algorithmic result as `<li class="b_algo"><h2><a href="...">...</a></h2>`.
//! Bing surfaces academic PDFs that DDG often misses (it indexes deeper into
//! .edu and government archives), so it's a useful complement.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

use super::web_common::{clean_title, looks_like_doc};
use super::{Document, Source, USER_AGENT};

const ENDPOINT: &str = "https://www.bing.com/search";
const MAX_PAGES: usize = 4;
const PAGE_SIZE: usize = 10; // Bing's `first` param uses 1, 11, 21, 31 …

pub struct BingHtmlSource {
    client: Arc<reqwest::Client>,
}

impl BingHtmlSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Match the result heading anchor inside `<li class="b_algo">` blocks.
// We don't anchor on `b_algo` because Bing sometimes wraps results in
// alternate containers; the `<h2><a href="...">...</a></h2>` shape is stable.
static RESULT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<h2[^>]*>\s*<a\s+[^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>\s*</h2>"#)
        .unwrap()
});

#[async_trait]
impl Source for BingHtmlSource {
    fn name(&self) -> &'static str {
        "bing"
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
                // Bing's `first` is 1-indexed: page 0 → first=1, page 1 → first=11.
                let first = (page * PAGE_SIZE + 1).to_string();
                let req = client
                    .get(ENDPOINT)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "text/html,application/xhtml+xml")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .query(&[("q", q.as_str()), ("first", first.as_str())])
                    .send()
                    .await;
                let resp = match req {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if !resp.status().is_success() {
                    return Some((
                        Err(anyhow::anyhow!("bing http {}", resp.status())),
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
                // only stop paginating on a genuinely empty page — not a page whose
                // results all happened to fail `looks_like_doc`.
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
                        source: "bing".to_string(),
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

    #[test]
    fn extracts_bing_h2_anchor() {
        let html = r#"
            <li class="b_algo">
              <h2><a href="https://stanford.edu/papers/civil-war.pdf" h="ID=SERP,5023.1">
                Civil War <strong>Primary Sources</strong>
              </a></h2>
            </li>
        "#;
        let cap = RESULT_RE.captures(html).expect("matches");
        assert_eq!(&cap[1], "https://stanford.edu/papers/civil-war.pdf");
        assert_eq!(
            clean_title(cap.get(2).unwrap().as_str()),
            "Civil War Primary Sources"
        );
    }
}
