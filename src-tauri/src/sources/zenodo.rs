//! Zenodo — CERN-hosted multidisciplinary open repository (papers, preprints,
//! theses, reports, conference proceedings). REST JSON, no key for search;
//! records carry direct file download URLs (high download conversion).
//!
//! Throttled to a low per-source concurrency (see `source_concurrency`): the
//! guest API rate-limits (~60/min). `get_with_retry` honors its `Retry-After`,
//! and a 429 only costs a backoff (API sources have no circuit breaker).

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use super::{send_with_retry, Document, Source};

const BASE: &str = "https://zenodo.org/api/records";
// Zenodo's (InvenioRDM) public API now caps the page size at 25 for
// unauthenticated requests — a `size` over 25 is a hard 400 ("Page size cannot
// be greater than 25"). Paginate in 25s instead; the source already walks pages.
const PAGE_SIZE: usize = 25;
/// Zenodo's CDN/WAF returns 403 Forbidden for the shared browser User-Agent
/// (an anti-scraping measure that flags fake-browser UAs). An honest tool UA +
/// `Accept: application/json` is served normally. See `search` below.
const ZENODO_UA: &str = "DocumentFinder/0.1 (open-access research tool)";

pub struct ZenodoSource {
    client: Arc<reqwest::Client>,
}

impl ZenodoSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Default, Deserialize)]
struct Resp {
    #[serde(default)]
    hits: Hits,
}

#[derive(Debug, Default, Deserialize)]
struct Hits {
    #[serde(default)]
    hits: Vec<Hit>,
}

#[derive(Debug, Deserialize)]
struct Hit {
    #[serde(default)]
    metadata: Meta,
    #[serde(default)]
    files: Vec<ZFile>,
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publication_date: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    creators: Vec<Creator>,
}

#[derive(Debug, Deserialize)]
struct Creator {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZFile {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    links: HashMap<String, String>,
}

/// Direct download URL for the first PDF/EPUB file in a record. InvenioRDM
/// exposes the content link under `self`; older shapes used `download`/`content`.
fn pick_file_url(files: &[ZFile]) -> Option<String> {
    let f = files.iter().find(|f| {
        f.key.as_deref().is_some_and(|k| {
            let k = k.to_lowercase();
            k.ends_with(".pdf") || k.ends_with(".epub")
        })
    })?;
    ["self", "download", "content"]
        .iter()
        .find_map(|k| f.links.get(*k).cloned())
}

/// Leading 4-digit year from an ISO-ish `publication_date`.
fn year_from_date(d: &str) -> Option<String> {
    d.get(..4)
        .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
        .map(String::from)
}

#[async_trait]
impl Source for ZenodoSource {
    fn name(&self) -> &'static str {
        "zenodo"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let q = keywords.join(" ");
        stream::unfold((1u32, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let params = [
                    ("q", q),
                    ("size", PAGE_SIZE.to_string()),
                    ("page", page.to_string()),
                    ("sort", "bestmatch".to_string()),
                    ("access_status", "open".to_string()),
                    ("type", "publication".to_string()),
                    ("type", "preprint".to_string()),
                ];
                let resp = match send_with_retry(BASE, || {
                    client
                        .get(BASE)
                        .query(&params)
                        .header(reqwest::header::USER_AGENT, ZENODO_UA)
                        .header(reqwest::header::ACCEPT, "application/json")
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (page, yielded, true))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if data.hits.hits.is_empty() {
                    return None;
                }
                let next_done = data.hits.hits.len() < PAGE_SIZE;
                let mut docs = Vec::new();
                for h in data.hits.hits {
                    let Some(url) = pick_file_url(&h.files) else {
                        continue;
                    };
                    let authors = h
                        .metadata
                        .creators
                        .into_iter()
                        .filter_map(|c| c.name)
                        .collect();
                    let year = h
                        .metadata
                        .publication_date
                        .as_deref()
                        .and_then(year_from_date);
                    docs.push(Document {
                        title: h.metadata.title.unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "zenodo".to_string(),
                        authors,
                        year,
                        abstract_: h.metadata.description,
                        identifier: h.metadata.doi,
                    });
                }
                // Count emitted docs toward the limit (not raw hits) so records
                // without a downloadable file don't starve deeper pages.
                let added = docs.len();
                Some((Ok(docs), (page + 1, yielded + added, next_done)))
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
    fn picks_pdf_file_self_link() {
        let mut links = HashMap::new();
        links.insert(
            "self".to_string(),
            "https://zenodo.org/api/records/1/files/p.pdf/content".to_string(),
        );
        let files = vec![
            ZFile {
                key: Some("data.csv".to_string()),
                links: HashMap::new(),
            },
            ZFile {
                key: Some("Paper.PDF".to_string()),
                links,
            },
        ];
        assert_eq!(
            pick_file_url(&files).as_deref(),
            Some("https://zenodo.org/api/records/1/files/p.pdf/content")
        );
    }

    #[test]
    fn no_document_file_is_none() {
        let files = vec![ZFile {
            key: Some("dataset.zip".to_string()),
            links: HashMap::new(),
        }];
        assert_eq!(pick_file_url(&files), None);
    }

    #[test]
    fn year_parses_iso_date() {
        assert_eq!(year_from_date("2021-06-15").as_deref(), Some("2021"));
        assert_eq!(year_from_date("n/a"), None);
    }
}
