pub mod arxiv;
pub mod bing_html;
pub mod brave_html;
pub mod doaj;
pub mod duckduckgo;
pub mod gutenberg;
pub mod internet_archive;
pub mod local_searxng;
pub mod marginalia_html;
pub mod meta_search;
pub mod mojeek_html;
pub mod openalex;
pub mod searxng_pool;
pub mod semantic_scholar;
pub mod startpage_html;
pub mod web_common;

use async_trait::async_trait;
use futures::stream::BoxStream;
use md5::{Digest, Md5};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::AppHandle;

/// Browser-shaped UA. Some publishers (Sage, OUP, Brill) 403 generic crawler
/// UAs even when the article is open access. We're not pretending to be a
/// browser to evade paywalls — just to stop being blocked at the gate.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// All known source ids. `meta_search` is the recommended zero-config web
/// backend; it fans out to the six individual web scrapers internally.
pub const SOURCE_IDS: &[&str] = &[
    "arxiv",
    "openalex",
    "semantic_scholar",
    "internet_archive",
    "doaj",
    "gutenberg",
    "meta_search",
    "searxng",
    "web",
    "brave",
    "bing",
    "mojeek",
    "marginalia",
    "startpage",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub title: String,
    pub url: String,
    pub source: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(rename = "abstract", default, skip_serializing_if = "Option::is_none")]
    pub abstract_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

static SLUG_NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\w\s-]").unwrap());
static SLUG_HYPHEN_RUNS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[-\s]+").unwrap());

impl Document {
    /// Filename-safe slug derived from title + 6-char URL hash. Mirrors Python's `slug()`.
    pub fn slug(&self) -> String {
        let cleaned = SLUG_NON_ALNUM.replace_all(&self.title, "");
        let trimmed = cleaned.trim();
        let with_hyphens = SLUG_HYPHEN_RUNS.replace_all(trimmed, "-");
        let mut base: String = with_hyphens.chars().take(80).collect();
        if base.is_empty() {
            base = "doc".to_string();
        }
        let mut hasher = Md5::new();
        hasher.update(self.url.as_bytes());
        let h = hex::encode(hasher.finalize());
        format!("{}-{}", base, &h[..6])
    }
}

/// Per-source configuration delivered from the frontend. Currently empty —
/// every backend builds itself from the shared HTTP client. Kept as an
/// extension point so adding per-source knobs later doesn't ripple through
/// every callsite of `build_source`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SourceOptions {}

#[async_trait]
pub trait Source: Send + Sync {
    fn name(&self) -> &'static str;

    /// Stream of `Document`s for the given keywords. Implementations honor `limit`
    /// internally and may stop early.
    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>>;
}

/// Build a source by id. Returns `None` for unknown ids.
pub fn build_source(
    name: &str,
    _options: SourceOptions,
    client: Arc<reqwest::Client>,
    app: Option<AppHandle>,
) -> Option<Box<dyn Source>> {
    match name {
        "arxiv" => Some(Box::new(arxiv::ArxivSource::new(client))),
        "openalex" => Some(Box::new(openalex::OpenAlexSource::new(client))),
        "semantic_scholar" => Some(Box::new(semantic_scholar::SemanticScholarSource::new(
            client,
        ))),
        "internet_archive" => Some(Box::new(internet_archive::InternetArchiveSource::new(
            client,
        ))),
        "doaj" => Some(Box::new(doaj::DOAJSource::new(client))),
        "gutenberg" => Some(Box::new(gutenberg::GutenbergSource::new(client))),
        "web" => Some(Box::new(duckduckgo::DuckDuckGoSource::new(client))),
        "brave" => Some(Box::new(brave_html::BraveHtmlSource::new(client))),
        "bing" => Some(Box::new(bing_html::BingHtmlSource::new(client))),
        "mojeek" => Some(Box::new(mojeek_html::MojeekHtmlSource::new(client))),
        "marginalia" => Some(Box::new(marginalia_html::MarginaliaHtmlSource::new(client))),
        "startpage" => Some(Box::new(startpage_html::StartpageHtmlSource::new(client))),
        "meta_search" => Some(Box::new(meta_search::MetaSearchSource::new(client, app))),
        // In-process SearXNG (prefers the local server, public pool fallback) —
        // a user-selectable, zero-setup source. No Docker or external instance.
        "searxng" => Some(Box::new(searxng_pool::SearxngPoolSource::new(client))),
        _ => None,
    }
}

pub fn make_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(60))
        .redirect(safe_redirect_policy())
        .build()
        .expect("http client")
}

/// Redirect policy that follows normal cross-host redirects but refuses to hop
/// to a non-http(s) scheme or to an IP-literal host in a private/reserved
/// range. This blocks the common SSRF-via-redirect cases (a public URL 302-ing
/// to `http://127.0.0.1` or the cloud-metadata `169.254.169.254`).
///
/// A redirect to a *hostname* that DNS-resolves to a private IP isn't caught
/// here — the policy closure is synchronous and can't resolve names. The
/// initial-hop `util::url_safety::validate_download_url` async check covers that
/// case for document downloads.
fn safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let url = attempt.url();
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return attempt.error("redirect to non-http(s) scheme blocked");
        }
        let private = match url.host() {
            Some(url::Host::Ipv4(ip)) => {
                crate::util::url_safety::is_private_ip(&std::net::IpAddr::V4(ip))
            }
            Some(url::Host::Ipv6(ip)) => {
                crate::util::url_safety::is_private_ip(&std::net::IpAddr::V6(ip))
            }
            _ => false,
        };
        if private {
            return attempt.error("redirect to a private/internal IP blocked");
        }
        attempt.follow()
    })
}

/// Helper: GET with exponential backoff on 429/5xx. Uses longer waits for
/// 429 specifically — APIs like Semantic Scholar set short rate-limit windows
/// that recover quickly if you back off enough.
pub async fn get_with_retry(
    client: &reqwest::Client,
    url: &str,
    query: &[(&str, String)],
) -> anyhow::Result<reqwest::Response> {
    let mut delay = std::time::Duration::from_millis(2000);
    for attempt in 1..=4 {
        let resp = client.get(url).query(query).send().await;
        match resp {
            Ok(r) => {
                let status = r.status().as_u16();
                if attempt < 4 && (status == 429 || (502..=504).contains(&status)) {
                    // 429 gets a longer wait — short retries just trip again.
                    let wait = if status == 429 {
                        std::time::Duration::from_secs(5 * attempt as u64 * attempt as u64)
                    } else {
                        delay
                    };
                    tracing::debug!("retry {} ({}): waiting {:?}", attempt, status, wait);
                    tokio::time::sleep(wait).await;
                    delay *= 2;
                    continue;
                }
                return Ok(r.error_for_status()?);
            }
            Err(e) if attempt < 4 => {
                tracing::debug!("retry {}: {}", attempt, e);
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
