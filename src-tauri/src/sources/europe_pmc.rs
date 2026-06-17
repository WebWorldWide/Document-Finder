//! Europe PMC — ~40M+ life-science abstracts and millions of open-access
//! full-text articles (PMC, repository mirrors, bioRxiv/medRxiv preprints).
//! REST JSON, cursorMark pagination, no API key.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

const BASE: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";

pub struct EuropePmcSource {
    client: Arc<reqwest::Client>,
}

impl EuropePmcSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default, rename = "nextCursorMark")]
    next_cursor_mark: Option<String>,
    #[serde(default, rename = "resultList")]
    result_list: ResultList,
}

#[derive(Debug, Default, Deserialize)]
struct ResultList {
    #[serde(default)]
    result: Vec<Hit>,
}

#[derive(Debug, Deserialize)]
struct Hit {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "authorString")]
    author_string: Option<String>,
    #[serde(default, rename = "pubYear")]
    pub_year: Option<String>,
    #[serde(default, rename = "abstractText")]
    abstract_text: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default, rename = "fullTextUrlList")]
    full_text_url_list: Option<FullTextUrlList>,
}

#[derive(Debug, Deserialize)]
struct FullTextUrlList {
    #[serde(default, rename = "fullTextUrl")]
    full_text_url: Vec<FullTextUrl>,
}

#[derive(Debug, Deserialize)]
struct FullTextUrl {
    #[serde(default, rename = "documentStyle")]
    document_style: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Pick the best downloadable URL for a hit: an explicit PDF full-text link
/// first, else the first full-text URL (a landing page the downloader resolves
/// to a PDF). The query already filters to `OPEN_ACCESS:Y`, and EPMC labels free
/// full text "Free"/"Open access", so we don't gate further on availability here.
/// `None` ⇒ no full-text URL at all, drop the hit.
fn pick_url(list: &FullTextUrlList) -> Option<String> {
    if let Some(url) = list
        .full_text_url
        .iter()
        .find(|u| {
            u.document_style
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
        })
        .and_then(|u| u.url.clone())
    {
        return Some(url);
    }
    list.full_text_url.iter().find_map(|u| u.url.clone())
}

#[async_trait]
impl Source for EuropePmcSource {
    fn name(&self) -> &'static str {
        "europe_pmc"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let query = format!("{} AND (OPEN_ACCESS:Y)", keywords.join(" "));
        // cursorMark "*" starts pagination; the API echoes the same mark on the
        // final page, which is the stop signal.
        stream::unfold((Some("*".to_string()), 0usize), move |(cursor, yielded)| {
            let client = client.clone();
            let query = query.clone();
            async move {
                let cur = cursor?;
                if yielded >= limit {
                    return None;
                }
                let page_size = limit.saturating_sub(yielded).clamp(1, 100);
                let params = [
                    ("query", query),
                    ("resultType", "core".to_string()),
                    ("format", "json".to_string()),
                    ("pageSize", page_size.to_string()),
                    ("cursorMark", cur.clone()),
                ];
                let resp = match get_with_retry(&client, BASE, &params).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (None, yielded))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (None, yielded))),
                };
                if data.result_list.result.is_empty() {
                    return None;
                }
                // Stop when the cursor stops advancing (EPMC echoes the mark).
                let next_cursor = match data.next_cursor_mark {
                    Some(n) if n != cur => Some(n),
                    _ => None,
                };
                let mut docs = Vec::new();
                for h in data.result_list.result {
                    let Some(url) = h.full_text_url_list.as_ref().and_then(pick_url) else {
                        continue;
                    };
                    let authors = h
                        .author_string
                        .as_deref()
                        .map(|s| {
                            s.split(',')
                                .map(|a| a.trim().trim_end_matches('.').trim().to_string())
                                .filter(|a| !a.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    docs.push(Document {
                        title: h.title.unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "europe_pmc".to_string(),
                        authors,
                        year: h.pub_year,
                        abstract_: h.abstract_text,
                        identifier: h.doi,
                    });
                }
                let added = docs.len();
                Some((Ok(docs), (next_cursor, yielded + added)))
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

    fn ft(style: Option<&str>, url: &str) -> FullTextUrl {
        FullTextUrl {
            document_style: style.map(String::from),
            url: Some(url.to_string()),
        }
    }

    #[test]
    fn prefers_pdf_then_first_full_text_url() {
        // PDF documentStyle wins even when an HTML link comes first.
        let list = FullTextUrlList {
            full_text_url: vec![
                ft(Some("html"), "https://x/landing"),
                ft(Some("pdf"), "https://x/paper.pdf"),
            ],
        };
        assert_eq!(pick_url(&list).as_deref(), Some("https://x/paper.pdf"));

        // No PDF → fall back to the first full-text URL (downloader resolves it).
        let no_pdf = FullTextUrlList {
            full_text_url: vec![ft(Some("html"), "https://x/landing")],
        };
        assert_eq!(pick_url(&no_pdf).as_deref(), Some("https://x/landing"));

        // No URLs at all → nothing to download.
        let none = FullTextUrlList {
            full_text_url: vec![],
        };
        assert_eq!(pick_url(&none), None);
    }
}
