//! arXiv — STEM preprints. Atom XML API. ≥3s pagination delay required by ToS.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use quick_xml::de::from_str;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use super::{Document, Source};

const BASE: &str = "https://export.arxiv.org/api/query";
const PAGINATION_DELAY: Duration = Duration::from_secs(3);

pub struct ArxivSource {
    client: Arc<reqwest::Client>,
}

impl ArxivSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Atom feeds carry top-level <link>, <id>, <title>, <updated> elements (for
// self / next / last navigation). quick-xml's serde adapter doesn't silently
// ignore them — it errors with `duplicate field 'link'` if the struct doesn't
// declare them. Declare and discard.
#[derive(Debug, Deserialize)]
struct Feed {
    #[serde(rename = "entry", default)]
    entries: Vec<Entry>,
    #[serde(rename = "link", default)]
    #[allow(dead_code)]
    feed_links: Vec<Link>,
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    title: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    published: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    updated: Option<String>,
    #[serde(rename = "author", default)]
    authors: Vec<Author>,
    #[serde(rename = "link", default)]
    links: Vec<Link>,
    // arXiv attaches multiple <category term="..."> elements per entry.
    #[serde(rename = "category", default)]
    #[allow(dead_code)]
    categories: Vec<Category>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Link {
    #[serde(rename = "@href", default)]
    href: Option<String>,
    #[serde(rename = "@title", default)]
    title: Option<String>,
    #[serde(rename = "@type", default)]
    type_: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Category {
    #[serde(rename = "@term", default)]
    term: Option<String>,
    #[serde(rename = "@scheme", default)]
    scheme: Option<String>,
}

fn entry_to_doc(entry: Entry) -> Option<Document> {
    let title = entry.title.unwrap_or_default().trim().to_string();
    let summary = entry.summary.unwrap_or_default().trim().to_string();
    let year = entry
        .published
        .as_deref()
        .and_then(|s| s.get(..4))
        .map(|s| s.to_string());
    let authors: Vec<String> = entry
        .authors
        .into_iter()
        .filter_map(|a| a.name)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut pdf_url: Option<String> = None;
    for link in &entry.links {
        let is_pdf = link.title.as_deref() == Some("pdf")
            || link.type_.as_deref() == Some("application/pdf");
        if is_pdf {
            pdf_url = link.href.clone();
            break;
        }
    }
    if pdf_url.is_none() {
        if let Some(id) = entry.id.as_deref() {
            if let Some(aid) = id.rsplit('/').next() {
                pdf_url = Some(format!("https://arxiv.org/pdf/{}.pdf", aid));
            }
        }
    }
    let url = pdf_url?;

    Some(Document {
        title,
        url,
        source: "arxiv".to_string(),
        authors,
        year: year.filter(|s| !s.is_empty()),
        abstract_: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        identifier: entry.id,
    })
}

#[async_trait]
impl Source for ArxivSource {
    fn name(&self) -> &'static str {
        "arxiv"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        stream::unfold((0usize, 0usize, false), move |(start, yielded, done)| {
            let client = client.clone();
            let keywords = keywords.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let per_page = 100.min(limit.saturating_sub(yielded).max(1));
                let q = keywords
                    .iter()
                    .map(|k| format!("all:{}", k))
                    .collect::<Vec<_>>()
                    .join("+AND+");
                let url = format!(
                    "{}?search_query={}&start={}&max_results={}",
                    BASE, q, start, per_page
                );
                let body = match client.get(&url).send().await {
                    Ok(r) => match r.error_for_status() {
                        Ok(r) => match r.text().await {
                            Ok(t) => t,
                            Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                        },
                        Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                    },
                    Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                };
                let feed: Feed = match from_str(&body) {
                    Ok(f) => f,
                    Err(e) => {
                        return Some((Err(anyhow::anyhow!("xml: {}", e)), (start, yielded, true)));
                    }
                };
                let n_entries = feed.entries.len();
                if n_entries == 0 {
                    return None;
                }
                let docs: Vec<Document> =
                    feed.entries.into_iter().filter_map(entry_to_doc).collect();
                let next_done = n_entries < per_page;
                if !next_done {
                    tokio::time::sleep(PAGINATION_DELAY).await;
                }
                Some((Ok(docs), (start + per_page, yielded + n_entries, next_done)))
            }
        })
        // Flatten: each step yields a Vec<Document> (or single Err).
        .flat_map(|res: anyhow::Result<Vec<Document>>| match res {
            Ok(docs) => stream::iter(docs.into_iter().map(Ok).collect::<Vec<_>>()).boxed(),
            Err(e) => stream::iter(vec![Err(e)]).boxed(),
        })
        .take(limit)
        .boxed()
    }
}
