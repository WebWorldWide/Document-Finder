//! DOAJ — Directory of Open Access Journals.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

// DOAJ's public API lives on the main host. The old `api.doaj.org` subdomain
// no longer resolves (NXDOMAIN), which surfaced as a connection error for every
// DOAJ sub-query. The query stays a URL path segment (DOAJ's documented shape).
const BASE: &str = "https://doaj.org/api/search/articles";

pub struct DOAJSource {
    client: Arc<reqwest::Client>,
}

impl DOAJSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    results: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    bibjson: Option<BibJson>,
}

#[derive(Debug, Deserialize)]
struct BibJson {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    year: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "abstract")]
    abstract_: Option<String>,
    #[serde(default)]
    author: Vec<BibAuthor>,
    #[serde(default)]
    link: Vec<BibLink>,
}

#[derive(Debug, Deserialize)]
struct BibAuthor {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BibLink {
    #[serde(default, rename = "type")]
    type_: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[async_trait]
impl Source for DOAJSource {
    fn name(&self) -> &'static str {
        "doaj"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let q = keywords.join(" AND ");
        let q_escaped = utf8_percent_encode(&q, NON_ALPHANUMERIC).to_string();
        let url = format!("{}/{}", BASE, q_escaped);
        // Page size MUST stay constant across pages: DOAJ offsets are
        // `(page-1)*pageSize`, so a pageSize that shrank as `yielded` grew would
        // re-anchor the window and skip results. (100 is the DOAJ API maximum.)
        let page_size = limit.clamp(1, 100);
        stream::unfold((1u32, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let url = url.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let params = [
                    ("page", page.to_string()),
                    ("pageSize", page_size.to_string()),
                ];
                let resp = match get_with_retry(&client, &url, &params).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (page, yielded, true))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if data.results.is_empty() {
                    return None;
                }
                let n = data.results.len();
                let next_done = n < page_size;
                let mut docs = Vec::with_capacity(n);
                for item in data.results {
                    let Some(bib) = item.bibjson else { continue };
                    let pdf = bib
                        .link
                        .iter()
                        .find(|l| {
                            l.type_.as_deref() == Some("fulltext")
                                && l.content_type
                                    .as_deref()
                                    .map(|s| s.to_lowercase().contains("pdf"))
                                    .unwrap_or(false)
                        })
                        .or_else(|| {
                            bib.link
                                .iter()
                                .find(|l| l.type_.as_deref() == Some("fulltext"))
                        });
                    let Some(url) = pdf.and_then(|l| l.url.clone()) else {
                        continue;
                    };
                    let year = match bib.year {
                        Some(serde_json::Value::String(s)) => Some(s),
                        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
                        _ => None,
                    };
                    let authors = bib
                        .author
                        .into_iter()
                        .filter_map(|a| a.name)
                        .collect::<Vec<_>>();
                    docs.push(Document {
                        title: bib.title.unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "doaj".to_string(),
                        authors,
                        year,
                        abstract_: bib.abstract_,
                        identifier: None,
                    });
                }
                Some((Ok(docs), (page + 1, yielded + n, next_done)))
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
