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
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

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

    let list: InstanceList = resp
        .json()
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

async fn get_instances(client: &reqwest::Client) -> Vec<String> {
    // Check cache under lock — clone if fresh.
    let cached = {
        let guard = INSTANCE_CACHE.lock();
        guard
            .as_ref()
            .filter(|c| c.fetched_at.elapsed() < CACHE_TTL)
            .cloned()
    };

    if let Some(c) = cached {
        return c.urls;
    }

    // Re-fetch outside lock.
    match fetch_instances(client).await {
        Ok(urls) => {
            let mut guard = INSTANCE_CACHE.lock();
            *guard = Some(CachedInstances {
                urls: urls.clone(),
                fetched_at: Instant::now(),
            });
            urls
        }
        Err(e) => {
            tracing::warn!("searxng_pool: failed to refresh instance list: {e}");
            // Return stale cache if available, otherwise empty.
            INSTANCE_CACHE
                .lock()
                .as_ref()
                .map(|c| c.urls.clone())
                .unwrap_or_default()
        }
    }
}

/// Issue a SearXNG query to the in-process local server (no SSRF check
/// needed — we control the URL and it is always 127.0.0.1).
async fn query_local(
    client: &reqwest::Client,
    base_url: &str,
    keywords: &str,
    limit: usize,
) -> anyhow::Result<Vec<Document>> {
    let search_url = format!("{base_url}/search");
    let resp = tokio::time::timeout(
        Duration::from_secs(15),
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
    .map_err(|_| anyhow::anyhow!("timeout querying local searxng"))?
    .map_err(|e| anyhow::anyhow!("local request: {e}"))?;

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

    let body: SearxResp = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("parse local searxng response: {e}"))?;

    Ok(body
        .results
        .into_iter()
        .take(limit)
        .map(|r| Document {
            title: r.title,
            url: r.url,
            source: "searxng_local".to_string(),
            authors: vec![],
            year: None,
            abstract_: r.content,
            identifier: None,
        })
        .collect())
}

/// Issue a SearXNG search query to one instance and return the raw results.
async fn query_instance(
    client: &reqwest::Client,
    base_url: &str,
    keywords: &str,
    limit: usize,
) -> anyhow::Result<Vec<Document>> {
    let search_url = format!("{base_url}/search");
    let resp = tokio::time::timeout(
        QUERY_TIMEOUT,
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

    let body: SearxResp = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("parse SearXNG response: {e}"))?;

    Ok(body
        .results
        .into_iter()
        .take(limit)
        .map(|r| Document {
            title: r.title,
            url: r.url,
            source: "searxng_pool".to_string(),
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
            match query_local(&self.client, &base_url, &query, limit).await {
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
            let second = (seed / n.max(1) + 1) % n;
            let mut picks = vec![instances[first].clone()];
            if n > 1 && second != first {
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
                match query_instance(&client, &url, &query, per_instance).await {
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
