pub mod arxiv;
pub mod bing_html;
pub mod brave_html;
pub mod doaj;
pub mod duckduckgo;
pub mod europe_pmc;
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
pub mod zenodo;

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
    "europe_pmc",
    "internet_archive",
    "doaj",
    "zenodo",
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

/// Per-source configuration delivered from the frontend. Currently empty — every
/// backend builds itself from the shared HTTP client. Kept as an extension point
/// so adding a per-source knob later doesn't ripple through every callsite of
/// `build_source`.
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
        "europe_pmc" => Some(Box::new(europe_pmc::EuropePmcSource::new(client))),
        "internet_archive" => Some(Box::new(internet_archive::InternetArchiveSource::new(
            client,
        ))),
        "doaj" => Some(Box::new(doaj::DOAJSource::new(client))),
        "zenodo" => Some(Box::new(zenodo::ZenodoSource::new(client))),
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
        // Drop private/reserved IPs at resolution time for every request and
        // redirect — closes the DNS-rebinding window and the private-hostname
        // redirect gap the synchronous policy can't (IP-literal hosts like the
        // in-process SearXNG bypass DNS and are unaffected).
        .dns_resolver(Arc::new(crate::util::url_safety::PublicOnlyResolver))
        .build()
        .expect("http client")
}

/// HTTP client tuned for document downloads. Unlike `make_client`, it sets NO
/// overall request timeout — reqwest's `.timeout()` is a *total* deadline
/// covering the whole body read, so a 60s cap silently aborts large or slow
/// PDFs mid-stream (and the partial file is then deleted, surfacing as a
/// failed download). Instead we bound only the parts that can legitimately
/// hang: `connect_timeout` fails fast on dead hosts, and `read_timeout` aborts
/// a stalled stream (no bytes for the window) while letting a slow-but-
/// progressing transfer run to completion.
pub fn make_download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(60))
        .redirect(safe_redirect_policy())
        // SSRF defence-in-depth: resolve to public IPs only, on every hop. See
        // `make_client` — this is the layer that survives DNS rebinding because
        // it sits in reqwest's own connection path, not a pre-flight lookup.
        .dns_resolver(Arc::new(crate::util::url_safety::PublicOnlyResolver))
        .build()
        .expect("download http client")
}

/// Redirect policy that follows normal cross-host redirects but refuses to hop
/// to a non-http(s) scheme or to an IP-literal host in a private/reserved
/// range. This blocks the common SSRF-via-redirect cases (a public URL 302-ing
/// to `http://127.0.0.1` or the cloud-metadata `169.254.169.254`).
///
/// This is the IP-literal layer; it can't resolve a redirect to a *hostname*
/// that maps to a private IP (the closure is synchronous). That case — and the
/// DNS-rebinding window between `validate_download_url` and the fetch — is now
/// covered by [`crate::util::url_safety::PublicOnlyResolver`], installed on both
/// HTTP clients, which filters private IPs inside reqwest's own resolution path
/// for every hop. The two layers are complementary defence-in-depth.
pub(crate) fn safe_redirect_policy() -> reqwest::redirect::Policy {
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

const RETRY_MAX_ATTEMPTS: u32 = 4;
/// Per-attempt backoff ceiling. The discovery wave aborts in-flight tasks at a
/// ~60s deadline; capping each sleep (and honoring it for a server-sent
/// `Retry-After`) keeps a single rate-limited source from sleeping past that
/// deadline and yielding zero docs. The old `5 * attempt^2` schedule reached
/// 5+20+45 = 70s, which blew the budget.
const RETRY_CAP: std::time::Duration = std::time::Duration::from_secs(20);

/// Parse a `Retry-After` header in its delta-seconds form. The HTTP-date form is
/// not handled (it falls back to the computed backoff); the keyless APIs we hit
/// (Semantic Scholar, archive.org) send delta-seconds.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
}

/// Additive jitter of up to `min(base, 3s)` on top of `base`, so concurrent
/// retries against the same rate-limited host don't wake in lockstep and
/// re-trip the limit. Adding (never subtracting) keeps a server-sent
/// `Retry-After` floor intact. Seeded from the wall clock + url + attempt to
/// decorrelate sibling tasks — the same dependency-free approach `searxng_pool`
/// uses, so we avoid pulling in `rand`.
fn backoff_with_jitter(base: std::time::Duration, url: &str, attempt: u32) -> std::time::Duration {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    url.hash(&mut h);
    attempt.hash(&mut h);
    let window_ms = (base.as_millis().min(3000)) as u64;
    let jitter = if window_ms == 0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_millis(h.finish() % (window_ms + 1))
    };
    base + jitter
}

/// Helper: GET with capped, jittered exponential backoff on 429/5xx. Honors a
/// server-sent `Retry-After` as a floor (capped at [`RETRY_CAP`]). 429 gets a
/// longer base wait — short retries against a rate-limit window just trip again.
pub async fn get_with_retry(
    client: &reqwest::Client,
    url: &str,
    query: &[(&str, String)],
) -> anyhow::Result<reqwest::Response> {
    let mut delay = std::time::Duration::from_millis(2000);
    for attempt in 1..=RETRY_MAX_ATTEMPTS {
        let resp = client.get(url).query(query).send().await;
        match resp {
            Ok(r) => {
                let status = r.status().as_u16();
                if attempt < RETRY_MAX_ATTEMPTS && (status == 429 || (502..=504).contains(&status))
                {
                    // Base wait: a server-sent Retry-After takes precedence;
                    // otherwise 429 gets a linear bump and 5xx the doubling delay.
                    let base = if status == 429 {
                        std::time::Duration::from_secs(5 * attempt as u64)
                    } else {
                        delay
                    };
                    let base = parse_retry_after(r.headers())
                        .unwrap_or(base)
                        .min(RETRY_CAP);
                    let wait = backoff_with_jitter(base, url, attempt);
                    tracing::debug!("retry {} ({}): waiting {:?}", attempt, status, wait);
                    tokio::time::sleep(wait).await;
                    delay = (delay * 2).min(RETRY_CAP);
                    continue;
                }
                return Ok(r.error_for_status()?);
            }
            Err(e) if attempt < RETRY_MAX_ATTEMPTS => {
                let wait = backoff_with_jitter(delay.min(RETRY_CAP), url, attempt);
                tracing::debug!("retry {}: {} (waiting {:?})", attempt, e, wait);
                tokio::time::sleep(wait).await;
                delay = (delay * 2).min(RETRY_CAP);
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
