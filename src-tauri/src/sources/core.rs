//! CORE — the largest aggregator of open-access research (~300M+ items) with
//! direct full-text PDF download URLs. **Opt-in**: requires a free API key
//! (pasted in Settings, threaded through `SourceOptions`). Off by default, so it
//! never affects the zero-config experience; with no key it contributes nothing.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

use super::{Document, Source};

const BASE: &str = "https://api.core.ac.uk/v3/search/works";
const PAGE_SIZE: usize = 100;

pub struct CoreSource {
    client: Arc<reqwest::Client>,
    api_key: Option<String>,
}

impl CoreSource {
    pub fn new(client: Arc<reqwest::Client>, api_key: Option<String>) -> Self {
        Self { client, api_key }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    results: Vec<Work>,
}

#[derive(Debug, Deserialize)]
struct Work {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    authors: Vec<Author>,
    #[serde(default, rename = "yearPublished")]
    year_published: Option<i64>,
    #[serde(default, rename = "abstract")]
    abstract_: Option<String>,
    #[serde(default, rename = "downloadUrl")]
    download_url: Option<String>,
    #[serde(default)]
    doi: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    name: Option<String>,
}

#[async_trait]
impl Source for CoreSource {
    fn name(&self) -> &'static str {
        "core"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        // No key configured → contribute nothing (never surfaces an error).
        let Some(key) = self.api_key.clone().filter(|k| !k.trim().is_empty()) else {
            return stream::empty().boxed();
        };
        let client = self.client.clone();
        let q = keywords.join(" ");
        stream::unfold((0usize, 0usize, false), move |(offset, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            let key = key.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let page = PAGE_SIZE.min(limit.saturating_sub(yielded)).max(1);
                let req = client
                    .get(BASE)
                    .header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"))
                    .query(&[
                        ("q", q.as_str()),
                        ("limit", &page.to_string()),
                        ("offset", &offset.to_string()),
                    ])
                    .send()
                    .await;
                let resp = match req.and_then(|r| r.error_for_status()) {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (offset, yielded, true))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (offset, yielded, true))),
                };
                if data.results.is_empty() {
                    return None;
                }
                let n = data.results.len();
                let next_done = n < page;
                let mut docs = Vec::new();
                for w in data.results {
                    let Some(url) = w.download_url.filter(|u| !u.is_empty()) else {
                        continue;
                    };
                    let authors = w.authors.into_iter().filter_map(|a| a.name).collect();
                    docs.push(Document {
                        title: w.title.unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "core".to_string(),
                        authors,
                        year: w.year_published.map(|y| y.to_string()),
                        abstract_: w.abstract_,
                        identifier: w.doi,
                    });
                }
                // Advance the raw offset by the page returned, count emitted docs.
                let added = docs.len();
                Some((Ok(docs), (offset + n, yielded + added, next_done)))
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
