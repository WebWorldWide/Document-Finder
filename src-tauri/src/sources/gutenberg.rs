//! Project Gutenberg — public-domain literature & classics, via gutendex.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use super::{Document, Source};

const SEARCH: &str = "https://gutendex.com/books";
const FORMAT_PRIORITY: &[&str] = &[
    "text/plain; charset=utf-8",
    "text/plain; charset=us-ascii",
    "text/plain",
    "application/epub+zip",
    "application/pdf",
];

pub struct GutenbergSource {
    client: Arc<reqwest::Client>,
}

impl GutenbergSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    results: Vec<Book>,
}

#[derive(Debug, Deserialize)]
struct Book {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    authors: Vec<BookAuthor>,
    #[serde(default)]
    formats: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct BookAuthor {
    #[serde(default)]
    name: Option<String>,
}

#[async_trait]
impl Source for GutenbergSource {
    fn name(&self) -> &'static str {
        "gutenberg"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let initial_url = {
            let q = keywords.join(" ");
            let mut u = url::Url::parse(SEARCH).expect("static url");
            u.query_pairs_mut().append_pair("search", &q);
            u.to_string()
        };
        stream::unfold((Some(initial_url), 0usize), move |(next_url, yielded)| {
            let client = client.clone();
            async move {
                let url = next_url?;
                if yielded >= limit {
                    return None;
                }
                // gutendex rate-limits; route through the shared retry helper so a
                // single transient 429/5xx backs off and retries instead of ending
                // the whole Gutenberg stream. It paginates by absolute `next` URL
                // (query already embedded), so we pass an empty query slice.
                let resp = match super::get_with_retry(&client, &url, &[]).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (None, yielded))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (None, yielded))),
                };
                let mut docs = Vec::new();
                let mut count = 0usize;
                for b in data.results {
                    if yielded + count >= limit {
                        break;
                    }
                    let file_url = FORMAT_PRIORITY
                        .iter()
                        .find_map(|k| b.formats.get(*k).cloned());
                    let Some(file_url) = file_url else { continue };
                    let authors = b
                        .authors
                        .into_iter()
                        .filter_map(|a| a.name)
                        .collect::<Vec<_>>();
                    docs.push(Document {
                        title: b.title.unwrap_or_else(|| "Untitled".to_string()),
                        url: file_url,
                        source: "gutenberg".to_string(),
                        authors,
                        year: None,
                        abstract_: None,
                        identifier: b.id.map(|i| i.to_string()),
                    });
                    count += 1;
                }
                Some((Ok(docs), (data.next, yielded + count)))
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
