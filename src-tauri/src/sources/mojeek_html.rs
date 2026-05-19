//! Zero-key web search via Mojeek's HTML results page.
//!
//! Mojeek runs its own crawler (not a Google/Bing front-end), so it
//! surfaces independent and academic sites that the mainstream engines
//! often miss. The results page renders each hit inside `<a class="ob">`
//! anchors that wrap the title and the canonical URL.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

use super::web_common::{clean_title, looks_like_doc};
use super::{Document, Source, USER_AGENT};

const ENDPOINT: &str = "https://www.mojeek.com/search";
const MAX_PAGES: usize = 4;
const PAGE_SIZE: usize = 10;

pub struct MojeekHtmlSource {
    client: Arc<reqwest::Client>,
}

impl MojeekHtmlSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Mojeek's main result anchor. The `class="ob"` opens the result card; the
// title is plain text inside the anchor.
static RESULT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?is)<a\s+[^>]*class="[^"]*\bob\b[^"]*"[^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap()
});

#[async_trait]
impl Source for MojeekHtmlSource {
    fn name(&self) -> &'static str {
        "mojeek"
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
                // Mojeek uses `s` for the start offset (1-indexed).
                let s = (page * PAGE_SIZE + 1).to_string();
                let req = client
                    .get(ENDPOINT)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "text/html,application/xhtml+xml")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .query(&[("q", q.as_str()), ("s", s.as_str())])
                    .send()
                    .await;
                let resp = match req {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if !resp.status().is_success() {
                    return Some((
                        Err(anyhow::anyhow!("mojeek http {}", resp.status())),
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
                        source: "mojeek".to_string(),
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
    fn extracts_mojeek_anchor() {
        let html = r#"
            <div>
              <a class="ob" href="https://example.edu/papers/x.pdf" data-rank="1">
                Example Paper Title
              </a>
            </div>
        "#;
        let cap = RESULT_RE.captures(html).expect("matches");
        assert_eq!(&cap[1], "https://example.edu/papers/x.pdf");
        assert_eq!(
            clean_title(cap.get(2).unwrap().as_str()),
            "Example Paper Title"
        );
    }
}
