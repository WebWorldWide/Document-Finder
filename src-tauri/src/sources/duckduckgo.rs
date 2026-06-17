//! Zero-setup web search via DuckDuckGo's HTML endpoint.
//!
//! Replaces the user-configurable SearXNG source. DDG's `html.duckduckgo.com/html/`
//! endpoint accepts a POST `q=` form and returns scrapeable HTML — no API key,
//! no instance to host.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use percent_encoding::percent_decode_str;
use regex::Regex;
use std::sync::Arc;

use super::web_common::{clean_title, looks_like_doc};
use super::{Document, Source, USER_AGENT};

const ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const MAX_PAGES: usize = 8;
const PAGE_SIZE: usize = 30;

pub struct DuckDuckGoSource {
    client: Arc<reqwest::Client>,
}

impl DuckDuckGoSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

// Match the result anchor in TWO stages — capture each anchor's attribute blob
// + inner HTML, then test for the `result__a` class and extract href SEPARATELY
// (mirrors brave_html.rs). The old single regex demanded a rigid attribute order
// (`rel` then `class` then `href`) and an EXACT single class, so any attribute
// reorder or an extra class token (e.g. `class="result__a js-result-title-link"`)
// silently matched nothing and DDG — the primary web engine — contributed zero.
static ANCHOR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?is)<a\s+([^>]*)>(.*?)</a>"#).unwrap());
static RESULT_CLASS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)class="[^"]*\bresult__a\b[^"]*""#).unwrap());
static HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)href="([^"]+)""#).unwrap());
fn unwrap_ddg_redirect(href: &str) -> String {
    // DDG wraps results as //duckduckgo.com/l/?uddg=<percent-encoded-url>&...
    let h = href.trim_start_matches("//");
    if let Some(idx) = h.find("uddg=") {
        let tail = &h[idx + 5..];
        let raw = tail.split('&').next().unwrap_or(tail);
        return percent_decode_str(raw).decode_utf8_lossy().into_owned();
    }
    if h.starts_with("http") {
        h.to_string()
    } else {
        format!("https://{h}")
    }
}

#[async_trait]
impl Source for DuckDuckGoSource {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        // Bias toward downloadable docs — same intent as the old SearXNG source.
        let q = format!("{} filetype:pdf OR filetype:epub", keywords.join(" "));

        stream::unfold((0usize, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done || yielded >= limit || page >= MAX_PAGES {
                    return None;
                }
                let s_offset = page * PAGE_SIZE;
                let form = [
                    ("q", q.as_str()),
                    ("kl", "us-en"),
                    ("s", &s_offset.to_string()),
                ];
                let req = client
                    .post(ENDPOINT)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "text/html")
                    .header("Referer", "https://html.duckduckgo.com/")
                    .form(&form)
                    .send()
                    .await;
                let resp = match req {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };
                if !resp.status().is_success() {
                    return Some((
                        Err(anyhow::anyhow!("ddg http {}", resp.status())),
                        (page, yielded, true),
                    ));
                }
                let body = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => return Some((Err(e.into()), (page, yielded, true))),
                };

                let mut docs: Vec<Document> = Vec::new();
                let mut count = 0usize;
                // Count RAW result matches separately from the filtered `docs`, so
                // we only stop paginating when a page parsed zero results — not
                // when a full page happened to be all landing pages that failed
                // `looks_like_doc` (the real docs may be on page 2/3).
                let mut raw = 0usize;
                for cap in ANCHOR_RE.captures_iter(&body) {
                    let attrs = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    if !RESULT_CLASS_RE.is_match(attrs) {
                        continue; // not a result anchor
                    }
                    raw += 1;
                    if yielded + count >= limit {
                        break;
                    }
                    let href = HREF_RE
                        .captures(attrs)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str())
                        .unwrap_or("");
                    let title_html = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                    let url = unwrap_ddg_redirect(href);
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
                        source: "web".to_string(),
                        authors: Vec::new(),
                        year: None,
                        abstract_: None,
                        identifier: None,
                    });
                    count += 1;
                }

                if raw == 0 {
                    return None; // zero raw results on this page → end of results
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
    fn unwraps_ddg_redirect() {
        let href =
            "//duckduckgo.com/l/?uddg=https%3A%2F%2Farxiv.org%2Fpdf%2F1706.03762.pdf&rut=abc";
        assert_eq!(
            unwrap_ddg_redirect(href),
            "https://arxiv.org/pdf/1706.03762.pdf"
        );
    }

    #[test]
    fn cleans_title_html() {
        assert_eq!(
            clean_title("Attention <b>Is All</b> You Need &amp; More"),
            "Attention Is All You Need & More"
        );
    }

    #[test]
    fn doc_extension_check() {
        assert!(looks_like_doc("https://example.com/paper.pdf"));
        assert!(looks_like_doc("https://example.com/book.epub"));
        assert!(looks_like_doc("https://example.com/paper.pdf?x=1"));
        assert!(!looks_like_doc("https://example.com/page.html"));
    }

    /// Extract (decoded-url, title) pairs exactly as the search loop does.
    fn extract(html: &str) -> Vec<(String, String)> {
        ANCHOR_RE
            .captures_iter(html)
            .filter_map(|cap| {
                let attrs = cap.get(1)?.as_str();
                if !RESULT_CLASS_RE.is_match(attrs) {
                    return None;
                }
                let href = HREF_RE.captures(attrs)?.get(1)?.as_str();
                Some((unwrap_ddg_redirect(href), clean_title(cap.get(2)?.as_str())))
            })
            .collect()
    }

    #[test]
    fn extracts_result_regardless_of_attribute_order_or_extra_classes() {
        // Attribute reorder + an extra class token must still parse (the old
        // rigid single-regex matched neither of these).
        let href_first = r#"<a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.edu%2Fp.pdf&rut=x" rel="nofollow" class="result__a js-result-title-link">Deep <b>Learning</b></a>"#;
        let class_extra = r#"<a rel="nofollow" class="result__a extra" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.edu%2Fp.pdf">Deep Learning</a>"#;
        for html in [href_first, class_extra] {
            let got = extract(html);
            assert_eq!(got.len(), 1, "failed to match: {html}");
            assert_eq!(got[0].0, "https://a.edu/p.pdf");
            assert_eq!(got[0].1, "Deep Learning");
        }
    }
}
