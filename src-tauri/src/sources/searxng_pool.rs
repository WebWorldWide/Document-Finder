//! SearXNG public-instance fallback pool.
//!
//! Used by MetaSearchSource when all primary HTML-scraper backends are
//! circuit-open. Fetches a curated list of public SearXNG instances from
//! searx.space (with a 24-hour TTL cache), randomly selects two healthy
//! instances, and fans out the query to them.
//!
//! SSRF safety: every instance URL is validated with `url_safety::validate_url`
//! before any request is issued.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::local_searxng;
use super::{Document, Source};
use crate::util::url_safety::validate_url;

const INSTANCES_URL: &str = "https://searx.space/data/instances.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// A zero-instance fetch (every instance filtered out, or transient DNS
/// failures) is only honored briefly — never for the full 24h — so one bad
/// refresh can't silently disable the last-resort web fallback for a day.
const EMPTY_CACHE_TTL: Duration = Duration::from_secs(60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Max bytes buffered from a public-instance JSON response. The instance list
/// and per-instance results come from third-party infrastructure (searx.space
/// mirrors chosen by TLS grade, not trust), so cap the body — a bare
/// `resp.json()` buffers the whole stream and a hostile/compromised instance
/// could send a huge one (memory-exhaustion DoS).
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

/// Read a response body up to [`MAX_JSON_BYTES`], then parse it as JSON. Fails
/// fast on an over-large declared `Content-Length` and hard-stops mid-stream if
/// an undeclared or under-declared body exceeds the cap.
async fn json_capped<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> anyhow::Result<T> {
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_JSON_BYTES {
            anyhow::bail!("response body too large: {len} bytes");
        }
    }
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("read body: {e}"))?;
        if buf.len() + chunk.len() > MAX_JSON_BYTES {
            anyhow::bail!("response body exceeded {MAX_JSON_BYTES} bytes");
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(|e| anyhow::anyhow!("parse JSON: {e}"))
}

#[derive(Clone)]
struct CachedInstances {
    urls: Vec<String>,
    fetched_at: Instant,
}

static INSTANCE_CACHE: Lazy<Mutex<Option<CachedInstances>>> = Lazy::new(|| Mutex::new(None));

// JSON shape from searx.space — only the fields we care about.
#[derive(Deserialize)]
struct InstanceList {
    instances: std::collections::HashMap<String, InstanceInfo>,
}

#[derive(Deserialize)]
struct InstanceInfo {
    #[serde(default)]
    network_type: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    tls: Option<TlsInfo>,
}

#[derive(Deserialize)]
struct TlsInfo {
    #[serde(default)]
    grade: String,
}

fn is_acceptable(url: &str, info: &InstanceInfo) -> bool {
    // Only normal (non-Tor, non-I2P) instances.
    if info.network_type != "normal" && !info.network_type.is_empty() {
        return false;
    }
    // Require TLS grade A or A+.
    let grade = info
        .tls
        .as_ref()
        .map(|t| t.grade.as_str())
        .unwrap_or("unknown");
    if grade != "A" && grade != "A+" {
        return false;
    }
    // Must be https.
    if !url.starts_with("https://") {
        return false;
    }
    // Skip very old versions (rough heuristic: major version < 1 is likely stale).
    if !info.version.is_empty() {
        let major: u32 = info
            .version
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if major < 1 {
            return false;
        }
    }
    true
}

async fn fetch_instances(client: &reqwest::Client) -> anyhow::Result<Vec<String>> {
    // Sync validation (no DNS lookup needed for a known HTTPS URL from our code).
    validate_url(INSTANCES_URL)
        .await
        .map_err(|e| anyhow::anyhow!("SSRF check on INSTANCES_URL: {e}"))?;

    let resp = tokio::time::timeout(Duration::from_secs(15), client.get(INSTANCES_URL).send())
        .await
        .map_err(|_| anyhow::anyhow!("timeout fetching instance list"))?
        .map_err(|e| anyhow::anyhow!("fetch failed: {e}"))?;

    let list: InstanceList = json_capped(resp)
        .await
        .map_err(|e| anyhow::anyhow!("parse instances.json: {e}"))?;

    let mut urls: Vec<String> = list
        .instances
        .iter()
        .filter(|(url, info)| is_acceptable(url, info))
        .map(|(url, _)| url.trim_end_matches('/').to_string())
        .collect();

    // Validate each URL with our SSRF checker (async DNS lookup).
    let mut validated = Vec::new();
    for url in &urls {
        match validate_url(url).await {
            Ok(_) => validated.push(url.clone()),
            Err(e) => tracing::debug!("pool: skipping instance {url}: {e}"),
        }
        if validated.len() >= 20 {
            break; // Keep a reasonable upper bound.
        }
    }
    urls = validated;

    tracing::info!("searxng_pool: {} validated instances cached", urls.len());
    Ok(urls)
}

/// Effective freshness window for a cache entry: a populated list is good for the
/// full TTL; an empty list expires quickly so a transient bad fetch self-heals.
fn cache_ttl_for(c: &CachedInstances) -> Duration {
    if c.urls.is_empty() {
        EMPTY_CACHE_TTL
    } else {
        CACHE_TTL
    }
}

async fn get_instances(client: &reqwest::Client) -> Vec<String> {
    // Check cache under lock — clone if fresh (short window for an empty list).
    let cached = {
        let guard = INSTANCE_CACHE.lock();
        guard
            .as_ref()
            .filter(|c| c.fetched_at.elapsed() < cache_ttl_for(c))
            .cloned()
    };

    if let Some(c) = cached {
        return c.urls;
    }

    // Re-fetch outside lock.
    match fetch_instances(client).await {
        Ok(urls) if !urls.is_empty() => {
            let mut guard = INSTANCE_CACHE.lock();
            *guard = Some(CachedInstances {
                urls: urls.clone(),
                fetched_at: Instant::now(),
            });
            urls
        }
        // A zero-instance result (Ok(empty)) or an error must NOT overwrite a
        // good cache with nothing. Prefer a still-valid populated stale list;
        // otherwise remember the empty for a SHORT window (EMPTY_CACHE_TTL) so we
        // re-probe soon instead of hammering searx.space every query.
        other => {
            match &other {
                Err(e) => tracing::warn!("searxng_pool: failed to refresh instance list: {e}"),
                Ok(_) => {
                    tracing::warn!("searxng_pool: instance refresh returned zero usable instances")
                }
            }
            let mut guard = INSTANCE_CACHE.lock();
            if let Some(c) = guard.as_ref() {
                if !c.urls.is_empty() && c.fetched_at.elapsed() < CACHE_TTL {
                    return c.urls.clone();
                }
            }
            *guard = Some(CachedInstances {
                urls: Vec::new(),
                fetched_at: Instant::now(),
            });
            Vec::new()
        }
    }
}

/// Issue a SearXNG `/search?format=json` query to `base_url` and map the
/// results to `Document`s tagged with `source_label`. Shared by the in-process
/// local server and the public pool instances — they speak the same API.
///
/// Callers are responsible for SSRF-validating `base_url` first; the local
/// server is exempt because it is always our own `127.0.0.1` address.
async fn query_searxng(
    client: &reqwest::Client,
    base_url: &str,
    keywords: &str,
    limit: usize,
    source_label: &'static str,
    timeout: Duration,
) -> anyhow::Result<Vec<Document>> {
    let search_url = format!("{base_url}/search");
    let resp = tokio::time::timeout(
        timeout,
        client
            .get(&search_url)
            .query(&[
                ("q", keywords),
                ("format", "json"),
                ("pageno", "1"),
                ("language", "en"),
            ])
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timeout querying {base_url}"))?
    .map_err(|e| anyhow::anyhow!("request to {base_url}: {e}"))?;

    // Inspect the HTTP status BEFORE parsing. Otherwise a 429 (rate-limited) or
    // 5xx with an HTML/Cloudflare body surfaces as a generic "parse SearXNG
    // response" error, masking a temporarily-throttled-but-healthy instance as a
    // broken one.
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!(
            "SearXNG instance {base_url} returned HTTP {}",
            status.as_u16()
        );
    }

    #[derive(Deserialize)]
    struct SearxResp {
        results: Vec<SearxResult>,
    }
    #[derive(Deserialize)]
    struct SearxResult {
        url: String,
        title: String,
        content: Option<String>,
    }

    let body: SearxResp = json_capped(resp)
        .await
        .map_err(|e| anyhow::anyhow!("parse SearXNG response from {base_url}: {e}"))?;

    Ok(body
        .results
        .into_iter()
        .take(limit)
        .map(|r| Document {
            title: r.title,
            url: r.url,
            source: source_label.to_string(),
            authors: vec![],
            year: None,
            abstract_: r.content,
            identifier: None,
        })
        .collect())
}

pub struct SearxngPoolSource {
    client: Arc<reqwest::Client>,
}

impl SearxngPoolSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Source for SearxngPoolSource {
    fn name(&self) -> &'static str {
        "searxng_pool"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let query = keywords.join(" ");

        // Prefer the in-process SearXNG-compatible server when available.
        // Bypasses public-instance fan-out entirely on success, which keeps
        // us off third-party infrastructure for the common case.
        if let Some(port) = local_searxng::local_port() {
            let base_url = format!("http://127.0.0.1:{port}");
            match query_searxng(
                &self.client,
                &base_url,
                &query,
                limit,
                "searxng_local",
                Duration::from_secs(15),
            )
            .await
            {
                Ok(docs) if !docs.is_empty() => {
                    tracing::debug!("searxng_pool: local server returned {} docs", docs.len());
                    return stream::iter(docs.into_iter().map(Ok)).boxed();
                }
                Ok(_) => tracing::debug!(
                    "searxng_pool: local server returned 0 docs; falling back to public pool"
                ),
                Err(e) => tracing::warn!(
                    "searxng_pool: local server query failed ({e}); falling back to public pool"
                ),
            }
        }

        let instances = get_instances(&self.client).await;

        if instances.is_empty() {
            tracing::warn!("searxng_pool: no instances available");
            return stream::empty().boxed();
        }

        // Randomly pick up to 2 instances.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut hasher);
        let seed = hasher.finish() as usize;

        let selected: Vec<String> = {
            let n = instances.len();
            let first = seed % n;
            let mut picks = vec![instances[first].clone()];
            if n > 1 {
                // Pick a GUARANTEED-distinct second instance for redundancy. The
                // old `(seed / n + 1) % n` could alias to `first` for many
                // seed/n pairs (e.g. n=2, even seed), silently halving coverage on
                // the fallback path. Offsetting `first` by 1..=n-1 can never equal
                // `first` (mod n).
                let second = (first + 1 + (seed >> 8) % (n - 1)) % n;
                picks.push(instances[second].clone());
            }
            picks
        };

        let (tx, rx) = mpsc::channel::<anyhow::Result<Document>>(64);
        let client = self.client.clone();
        let query_clone = query.clone();
        let per_instance = limit.max(8);

        for url in selected {
            let tx = tx.clone();
            let client = client.clone();
            let query = query_clone.clone();
            tokio::spawn(async move {
                match query_searxng(
                    &client,
                    &url,
                    &query,
                    per_instance,
                    "searxng_pool",
                    QUERY_TIMEOUT,
                )
                .await
                {
                    Ok(docs) => {
                        for doc in docs {
                            if tx.send(Ok(doc)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("searxng_pool: instance {} failed: {}", url, e);
                    }
                }
            });
        }
        drop(tx);

        let seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        stream::unfold((rx, seen), |(mut rx, mut seen)| async move {
            while let Some(item) = rx.recv().await {
                if let Ok(ref doc) = item {
                    let key = doc.url.to_lowercase();
                    if !seen.insert(key) {
                        continue;
                    }
                }
                return Some((item, (rx, seen)));
            }
            None
        })
        .take(limit)
        .boxed()
    }
}
