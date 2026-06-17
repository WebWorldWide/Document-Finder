//! Semantic Scholar — ~200M papers, OA PDFs.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

const BASE: &str = "https://api.semanticscholar.org/graph/v1/paper/search";

pub struct SemanticScholarSource {
    client: Arc<reqwest::Client>,
}

impl SemanticScholarSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    data: Vec<Paper>,
    #[serde(default)]
    next: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Paper {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    authors: Vec<Author>,
    #[serde(default)]
    year: Option<i64>,
    #[serde(default)]
    #[serde(rename = "abstract")]
    abstract_: Option<String>,
    #[serde(default, rename = "openAccessPdf")]
    open_access_pdf: Option<OpenAccessPdf>,
    #[serde(default, rename = "externalIds")]
    external_ids: Option<ExternalIds>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAccessPdf {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExternalIds {
    #[serde(default, rename = "DOI")]
    doi: Option<String>,
}

#[async_trait]
impl Source for SemanticScholarSource {
    fn name(&self) -> &'static str {
        "semantic_scholar"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let query = keywords.join(" ");
        stream::unfold((0i64, 0usize, false), move |(offset, yielded, done)| {
            let client = client.clone();
            let query = query.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                // Semantic Scholar rate-limits aggressively without an API
                // key. 50 per page is the sweet spot between coverage and
                // not getting 429'd on the first request.
                let per_page = 50.min(limit.saturating_sub(yielded).max(1));
                let params = [
                    ("query", query),
                    ("limit", per_page.to_string()),
                    ("offset", offset.to_string()),
                    (
                        "fields",
                        "title,authors,year,abstract,openAccessPdf,externalIds".to_string(),
                    ),
                ];
                let resp = match get_with_retry(&client, BASE, &params).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (offset, yielded, true))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (offset, yielded, true))),
                };
                if data.data.is_empty() {
                    return None;
                }
                let n = data.data.len();
                let next_done = data.next.is_none() && n < per_page;
                let mut docs = Vec::with_capacity(n);
                for p in data.data {
                    let Some(url) = p.open_access_pdf.and_then(|o| o.url) else {
                        continue;
                    };
                    let authors: Vec<String> =
                        p.authors.into_iter().filter_map(|a| a.name).collect();
                    docs.push(Document {
                        title: p.title.unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "semantic_scholar".to_string(),
                        authors,
                        year: p.year.map(|y| y.to_string()),
                        abstract_: p.abstract_,
                        identifier: p.external_ids.and_then(|e| e.doi),
                    });
                }
                // Advance the *raw* offset by the page we requested (or the API's
                // own `next`), but count only EMITTED docs toward the limit. Most
                // S2 results lack an openAccessPdf and are dropped above; counting
                // raw results here would burn the budget on non-OA papers and stop
                // the stream long before `limit` downloadable PDFs were collected.
                let added = docs.len();
                let next_offset = data.next.unwrap_or(offset + per_page as i64);
                Some((Ok(docs), (next_offset, yielded + added, next_done)))
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
