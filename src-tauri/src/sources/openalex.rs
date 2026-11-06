//! OpenAlex — ~250M scholarly works, OA-filtered, cursor-paginated.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

const BASE: &str = "https://api.openalex.org/works";

/// Only the fields we deserialize, to keep the response small (OpenAlex returns
/// the full work object otherwise). `abstract_inverted_index` is requested so we
/// can reconstruct the abstract — without it every OpenAlex doc would carry a
/// title only, and any relevant work whose title lacks the literal query tokens
/// scores TF-IDF 0 and gets dropped before download.
const SELECT: &str = "id,doi,title,publication_year,best_oa_location,primary_location,locations,\
authorships,abstract_inverted_index,cited_by_count";

/// Project contact for OpenAlex's "polite pool" (faster, more reliable
/// responses). A project address — never the user's.
const POLITE_CONTACT: &str = "documentfinder@webworldwide.com";

/// Hosts that redirect PDF URLs to HTML landing pages.
const REDIRECTOR_HOSTS: &[&str] = &["hdl.handle.net", "doi.org", "dx.doi.org", "purl.org"];

fn is_likely_pdf_url(url: &str) -> bool {
    let u = url.to_lowercase();
    if REDIRECTOR_HOSTS.iter().any(|h| u.contains(h)) {
        return false;
    }

    // We're now more permissive. If it looks like a PDF or a common repository
    // download link, we'll try it. The downloader's content-type and magic-byte
    // checks will catch any non-PDFs that slip through.
    if u.contains(".pdf")
        || u.contains("/pdf/")
        || u.contains("/pdf?")
        || u.contains("download=pdf")
    {
        return true;
    }

    if u.contains("/bitstream/") || u.contains("/download/") || u.contains("getfile") {
        return true;
    }

    // If it's a direct link that doesn't explicitly claim to be HTML or an
    // abstract page, give it a shot. We deliberately do NOT reject `/article/`:
    // major OA publishers serve their direct PDF from an `/article/` path (e.g.
    // PLOS `…/article/file?id=…&type=printable`), and the downloader's
    // content-type / magic-byte check still gates non-PDFs.
    !u.ends_with(".html") && !u.ends_with(".htm") && !u.contains("/abs/")
}

pub struct OpenAlexSource {
    client: Arc<reqwest::Client>,
}

impl OpenAlexSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    results: Vec<Work>,
    #[serde(default)]
    meta: Option<Meta>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Work {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publication_year: Option<i64>,
    #[serde(default)]
    best_oa_location: Option<Location>,
    #[serde(default)]
    primary_location: Option<Location>,
    #[serde(default)]
    locations: Vec<Location>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    cited_by_count: Option<u32>,
    #[serde(default)]
    authorships: Vec<Authorship>,
    #[serde(default)]
    abstract_inverted_index: Option<HashMap<String, Vec<i64>>>,
}

/// Reconstruct plain-text abstract from OpenAlex's inverted index
/// (`word -> [token positions]`). Returns `None` for an empty/blank result.
fn reconstruct_abstract(inverted: &HashMap<String, Vec<i64>>) -> Option<String> {
    let max_pos = inverted
        .values()
        .flatten()
        .copied()
        .filter(|&p| p >= 0)
        .max()? as usize;
    // Guard against a pathological index claiming an enormous position.
    if max_pos > 100_000 {
        return None;
    }
    let mut words: Vec<Option<&str>> = vec![None; max_pos + 1];
    for (word, positions) in inverted {
        for &p in positions {
            if let Some(slot) = words.get_mut(p as usize) {
                *slot = Some(word.as_str());
            }
        }
    }
    let text = words.into_iter().flatten().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[derive(Debug, Deserialize)]
struct Location {
    #[serde(default)]
    pdf_url: Option<String>,
    #[serde(default)]
    landing_page_url: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    is_oa: Option<bool>,
    #[serde(default)]
    source: Option<LocSource>,
}

#[derive(Debug, Deserialize)]
struct LocSource {
    #[serde(default)]
    display_name: Option<String>,
}

/// Every candidate URL for a work, best first: direct PDF links from the best
/// OA location, the primary location and every other OA location, then OA
/// landing pages (the downloader resolves `citation_pdf_url` from them).
fn candidate_urls(w: &Work) -> Vec<String> {
    let locs: Vec<&Location> = w
        .best_oa_location
        .iter()
        .chain(w.primary_location.iter())
        .chain(w.locations.iter())
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut push = |u: &str| {
        if !u.is_empty() && !out.iter().any(|o| o == u) {
            out.push(u.to_string());
        }
    };
    for l in &locs {
        if let Some(u) = l.pdf_url.as_deref().filter(|u| is_likely_pdf_url(u)) {
            push(u);
        }
    }
    for l in &locs {
        if l.is_oa != Some(false) {
            if let Some(u) = l.landing_page_url.as_deref() {
                push(u);
            }
        }
    }
    out.truncate(6);
    out
}

#[derive(Debug, Deserialize)]
struct Authorship {
    #[serde(default)]
    author: Option<Author>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    display_name: Option<String>,
}

#[async_trait]
impl Source for OpenAlexSource {
    fn name(&self) -> &'static str {
        "openalex"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        let search = keywords.join(" ");
        stream::unfold((Some("*".to_string()), 0usize), move |(cursor, yielded)| {
            let client = client.clone();
            let search = search.clone();
            async move {
                let cur = cursor?;
                if yielded >= limit {
                    return None;
                }
                let per_page = 200.min(limit.saturating_sub(yielded).max(1));
                let params = [
                    ("search", search),
                    ("per-page", per_page.to_string()),
                    // `is_oa:true` guarantees an OA location. We dropped the old
                    // `has_fulltext:true` — that is OpenAlex's own n-gram index
                    // flag, not "has a downloadable PDF", and it excluded many OA
                    // works that do have a valid pdf_url. Local PDF + content-type
                    // checks still gate quality.
                    ("filter", "is_oa:true".to_string()),
                    ("select", SELECT.to_string()),
                    ("cursor", cur),
                    ("mailto", POLITE_CONTACT.to_string()),
                ];
                let resp = match get_with_retry(&client, BASE, &params).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), (None, yielded))),
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => return Some((Err(e.into()), (None, yielded))),
                };
                if data.results.is_empty() {
                    return None;
                }
                let next_cursor = data.meta.and_then(|m| m.next_cursor);
                let mut docs = Vec::with_capacity(data.results.len());
                for w in data.results {
                    let mut urls = candidate_urls(&w);
                    if urls.is_empty() {
                        continue;
                    }
                    let url = urls.remove(0);
                    let venue = w
                        .primary_location
                        .as_ref()
                        .and_then(|l| l.source.as_ref())
                        .and_then(|s| s.display_name.clone());
                    let license = w.best_oa_location.as_ref().and_then(|l| l.license.clone());
                    let doi = w.doi.as_deref().and_then(crate::engine::dedup::extract_doi);
                    let authors: Vec<String> = w
                        .authorships
                        .iter()
                        .filter_map(|a| a.author.as_ref().and_then(|x| x.display_name.clone()))
                        .collect();
                    let abstract_ = w
                        .abstract_inverted_index
                        .as_ref()
                        .and_then(reconstruct_abstract);
                    docs.push(Document {
                        title: w.title.clone().unwrap_or_else(|| "Untitled".to_string()),
                        url,
                        source: "openalex".to_string(),
                        authors,
                        year: w.publication_year.map(|y| y.to_string()),
                        abstract_,
                        identifier: w.id.clone(),
                        doi,
                        alt_urls: urls,
                        cited_by: w.cited_by_count,
                        venue,
                        license,
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

    #[test]
    fn rejects_handle_resolvers() {
        assert!(!is_likely_pdf_url(
            "http://hdl.handle.net/11858/00-001M-0000"
        ));
        assert!(!is_likely_pdf_url("https://doi.org/10.1234/x"));
        assert!(!is_likely_pdf_url("https://dx.doi.org/10.1234/x"));
    }

    #[test]
    fn accepts_pdf_paths() {
        assert!(is_likely_pdf_url("https://arxiv.org/pdf/1706.03762.pdf"));
        assert!(is_likely_pdf_url("https://example.org/pdf/123"));
        assert!(is_likely_pdf_url("https://example.org/file?download=pdf"));
    }

    #[test]
    fn accepts_potential_landing_pages_for_download_attempt() {
        // We now allow these so the downloader can check the Content-Type.
        assert!(is_likely_pdf_url(
            "https://digitalcommons.andrews.edu/auss/vol29/iss1/24"
        ));
        assert!(is_likely_pdf_url(
            "https://www.mdpi.com/2076-0760/6/1/5/pdf?version=1484055587"
        ));
    }

    #[test]
    fn accepts_publisher_article_pdf_paths() {
        // Major OA publishers serve the direct PDF from an `/article/` path;
        // these must no longer be rejected (the downloader validates content-type).
        assert!(is_likely_pdf_url(
            "https://journals.plos.org/plosone/article/file?id=10.1371/journal.pone.0000000&type=printable"
        ));
        assert!(is_likely_pdf_url(
            "https://link.springer.com/content/pdf/10.1007/s00000-000-0000-0.pdf"
        ));
    }

    #[test]
    fn reconstructs_abstract_from_inverted_index() {
        let mut inv: HashMap<String, Vec<i64>> = HashMap::new();
        inv.insert("Deep".to_string(), vec![0]);
        inv.insert("learning".to_string(), vec![1, 3]);
        inv.insert("for".to_string(), vec![2]);
        inv.insert("health".to_string(), vec![4]);
        assert_eq!(
            reconstruct_abstract(&inv).as_deref(),
            Some("Deep learning for learning health")
        );
    }

    #[test]
    fn reconstruct_abstract_empty_is_none() {
        let inv: HashMap<String, Vec<i64>> = HashMap::new();
        assert!(reconstruct_abstract(&inv).is_none());
    }
}
