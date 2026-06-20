//! Internet Archive — millions of books, esp. humanities & public domain.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

const SEARCH: &str = "https://archive.org/advancedsearch.php";

/// A candidate from an IA search page, before its real download URL is resolved:
/// (identifier, title, authors, year, description). Resolution is one extra
/// metadata fetch per item, done downstream so it doesn't gate the whole page.
type RawCandidate = (String, String, Vec<String>, Option<String>, Option<String>);

pub struct InternetArchiveSource {
    client: Arc<reqwest::Client>,
}

impl InternetArchiveSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    response: Inner,
}

#[derive(Debug, Default, Deserialize)]
struct Inner {
    #[serde(default)]
    docs: Vec<Value>,
}

fn coerce_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn coerce_string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(default)]
    files: Vec<MetaFile>,
}

#[derive(Debug, Deserialize)]
struct MetaFile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

/// Find the first file of a given document type (`ext`, e.g. "pdf"/"epub") in an
/// IA item's file list — by format string first, then by filename suffix.
fn find_file<'a>(files: &'a [MetaFile], ext: &str) -> Option<&'a str> {
    let suffix = format!(".{ext}");
    files
        .iter()
        .find(|f| {
            f.format
                .as_deref()
                .is_some_and(|s| s.to_lowercase().contains(ext))
        })
        .or_else(|| {
            files.iter().find(|f| {
                f.name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase().ends_with(&suffix))
            })
        })
        .and_then(|f| f.name.as_deref())
}

/// Resolve the real downloadable document URL for an Internet Archive item via
/// its metadata API. The bulk-search API only returns the item *identifier*; the
/// old guess of `{ident}/{ident}.pdf` 404s for the many items (arXiv/PubMed
/// mirrors, scanned books) whose primary file is named something else. We ask
/// `/metadata/{ident}` for the actual file list and prefer a PDF, falling back
/// to an EPUB (extract.rs handles both) — many public-domain / humanities books
/// on IA are EPUB-only. Returns `None` (item dropped) on any error or if no
/// usable document file exists, so a flaky lookup never blocks discovery.
async fn resolve_doc_url(client: &reqwest::Client, ident: &str) -> Option<String> {
    let meta_url = format!("https://archive.org/metadata/{ident}");
    // Route through get_with_retry (NOT a bare send) so a 429/Retry-After or 5xx
    // from archive.org/metadata is backed off and retried instead of silently
    // dropping the candidate. Under the broad multi-sub-query fan-out a single
    // discovery wave fires dozens of concurrent metadata lookups and IA
    // rate-limits them, which otherwise made matched books vanish with no visible
    // reason (the source still reported "done"). A genuine 404/non-doc still maps
    // to None and is legitimately dropped.
    let resp = get_with_retry(client, &meta_url, &[]).await.ok()?;
    let meta: Metadata = resp.json().await.ok()?;
    let name = find_file(&meta.files, "pdf").or_else(|| find_file(&meta.files, "epub"))?;
    // Build the URL with proper path-segment encoding (file names can contain
    // spaces or other reserved characters).
    let mut u = url::Url::parse("https://archive.org/download/").ok()?;
    u.path_segments_mut().ok()?.push(ident).push(name);
    Some(u.to_string())
}

#[async_trait]
impl Source for InternetArchiveSource {
    fn name(&self) -> &'static str {
        "internet_archive"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        // A second clone for the downstream per-item metadata resolution (the
        // unfold below moves `client` in for its paginated search requests).
        let resolve_client = self.client.clone();
        // IA's format field uses values like "Text PDF" — `format:pdf` matched
        // very few items. Filter to texts only and let the metadata lookup
        // confirm a PDF exists.
        let q = format!("{} AND mediatype:texts", keywords.join(" AND "));
        stream::unfold((1u32, false), move |(page, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done {
                    return None;
                }
                // Constant across pages: IA paginates by page *number* with a fixed
                // `rows`, so a per-page size that varied between pages would misalign
                // the offsets and skip results. Raised from a flat 50 so deep runs
                // pull more per page. The total count of RESOLVED docs is bounded by
                // `.take(limit)` downstream (which also halts pagination once it has
                // enough), so this stream stops on either that or a short last page.
                let per_page: usize = limit.clamp(50, 100);
                let params = [
                    ("q", q),
                    ("fl[]", "identifier".to_string()),
                    ("fl[]", "title".to_string()),
                    ("fl[]", "creator".to_string()),
                    ("fl[]", "year".to_string()),
                    ("fl[]", "description".to_string()),
                    ("fl[]", "format".to_string()),
                    ("rows", per_page.to_string()),
                    ("page", page.to_string()),
                    ("output", "json".to_string()),
                    ("sort[]", "downloads desc".to_string()),
                ];
                let resp = match get_with_retry(&client, SEARCH, &params).await {
                    Ok(r) => r,
                    Err(e) => {
                        return Some((
                            Err(anyhow::anyhow!("archive.org search failed: {e}")),
                            (page, true),
                        ));
                    }
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => {
                        return Some((
                            Err(anyhow::anyhow!("archive.org returned malformed JSON: {e}")),
                            (page, true),
                        ));
                    }
                };
                let docs_in = data.response.docs;
                if docs_in.is_empty() {
                    return None;
                }
                let n = docs_in.len();
                let next_done = n < per_page;
                // Collect raw candidates (items advertising a PDF/EPUB). Their real
                // download URLs are resolved DOWNSTREAM (in flat_map), one metadata
                // lookup per item yielded as it resolves — so a single slow/retried
                // lookup delays only its own doc, not the whole page. A
                // collect()-before-yield here would gate every doc on the page's
                // slowest lookup, risking loss of the entire page if the wave
                // deadline fires mid-resolution.
                let mut raw: Vec<RawCandidate> = Vec::with_capacity(n);
                for d in docs_in {
                    let Some(ident) = d.get("identifier").and_then(coerce_string) else {
                        continue;
                    };
                    // Filter to items whose format list mentions PDF or EPUB —
                    // avoids metadata lookups for items that only have image scans
                    // or DjVu, while keeping EPUB-only books (common on IA).
                    let formats = d
                        .get("format")
                        .map(coerce_string_list)
                        .unwrap_or_default()
                        .join(" ")
                        .to_lowercase();
                    if !formats.contains("pdf") && !formats.contains("epub") {
                        continue;
                    }
                    let authors = d.get("creator").map(coerce_string_list).unwrap_or_default();
                    let desc = d.get("description").and_then(coerce_string);
                    let title = d
                        .get("title")
                        .and_then(coerce_string)
                        .unwrap_or_else(|| ident.clone());
                    let year = d.get("year").and_then(coerce_string);
                    raw.push((ident, title, authors, year, desc));
                }
                Some((Ok(raw), (page + 1, next_done)))
            }
        })
        // Resolve each candidate's real download URL here, incrementally: every
        // item is yielded as soon as its own metadata lookup finishes, so the wave
        // deadline cutting in mid-page only loses still-pending items, not the ones
        // already resolved.
        .flat_map(move |res: anyhow::Result<Vec<RawCandidate>>| match res {
            Ok(raw) => {
                let client = resolve_client.clone();
                stream::iter(raw)
                    .map(move |(ident, title, authors, year, desc)| {
                        let client = client.clone();
                        async move {
                            let url = resolve_doc_url(&client, &ident).await?;
                            Some(Document {
                                title,
                                url,
                                source: "internet_archive".to_string(),
                                authors,
                                year,
                                abstract_: desc,
                                identifier: Some(ident),
                            })
                        }
                    })
                    .buffer_unordered(8)
                    .filter_map(|d| async move { d.map(Ok) })
                    .boxed()
            }
            Err(e) => stream::iter(vec![Err(e)]).boxed(),
        })
        .take(limit)
        .boxed()
    }
}
