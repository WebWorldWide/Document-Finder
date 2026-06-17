//! Built-in meta-search aggregator — the zero-config replacement for SearXNG.
//!
//! Fans out the query to all six HTML web scrapers concurrently, runs a
//! circuit breaker per backend so repeatedly failing engines are skipped for
//! 5 minutes, and falls back to public SearxNG instances when all circuits
//! are open. Emits `df:meta_search_health` after each backend completes so
//! the UI can show a per-engine health badge.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use super::bing_html::BingHtmlSource;
use super::brave_html::BraveHtmlSource;
use super::duckduckgo::DuckDuckGoSource;
use super::marginalia_html::MarginaliaHtmlSource;
use super::mojeek_html::MojeekHtmlSource;
use super::searxng_pool::SearxngPoolSource;
use super::startpage_html::StartpageHtmlSource;
use super::{Document, Source};
use crate::events::{MetaSearchHealthPayload, EV_META_SEARCH_HEALTH};

/// Per-engine wall-clock budget within one fan-out, scaled with the requested
/// `limit` (per_source) so a deeper run lets each scraper paginate further before
/// it's cut off. Bounded at 20s so one slow engine can't stall the aggregator
/// (which itself runs under the orchestrator's depth-scaled wave deadline).
///   limit  25 (Light)      →  10s
///   limit 100 (Balanced)   →  16s
///   limit 200+ (Deep/Exh.) →  20s (clamped)
fn backend_timeout(limit: usize) -> Duration {
    Duration::from_secs((8 + limit / 12).clamp(8, 20) as u64)
}
const CIRCUIT_OPEN_FAILURES: u32 = 3;
const CIRCUIT_OPEN_DURATION: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
struct BackendHealth {
    consecutive_failures: u32,
    skip_until: Option<Instant>,
}

impl BackendHealth {
    fn is_open(&self) -> bool {
        if let Some(until) = self.skip_until {
            Instant::now() < until
        } else {
            false
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.skip_until = None;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= CIRCUIT_OPEN_FAILURES {
            self.skip_until = Some(Instant::now() + CIRCUIT_OPEN_DURATION);
        }
    }
}

/// How one engine's fan-out maps onto the circuit breaker. A zero-result query is
/// `Neutral` — only real transport/HTTP errors count toward opening a circuit, so
/// a legitimately empty (or anti-bot-throttled) engine is never suppressed.
enum Outcome {
    Success,
    Failure,
    Neutral,
}

static HEALTH: Lazy<Mutex<HashMap<&'static str, BackendHealth>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

struct EngineEntry {
    name: &'static str,
    source: Box<dyn Source>,
}

pub struct MetaSearchSource {
    engines: Vec<EngineEntry>,
    app: Option<AppHandle>,
    fallback: Option<Arc<SearxngPoolSource>>,
}

impl MetaSearchSource {
    /// Standard aggregator: six web scrapers with the public SearXNG pool as a
    /// fallback when every scraper's circuit is open.
    pub fn new(client: Arc<reqwest::Client>, app: Option<AppHandle>) -> Self {
        let engines = Self::build_engines(&client);
        let fallback = Some(Arc::new(SearxngPoolSource::new(client)));
        Self {
            engines,
            app,
            fallback,
        }
    }

    /// Variant with **no** public-pool fallback. Used to back the in-process
    /// SearXNG server (`local_searxng`): the pool prefers that same local
    /// server, so a pool-backed aggregator there would recurse local → pool →
    /// local indefinitely. Without a fallback the local server returns an empty
    /// page when all circuits are open, and the real pool fan-out runs one
    /// level up in the caller.
    pub fn new_without_pool_fallback(client: Arc<reqwest::Client>, app: Option<AppHandle>) -> Self {
        let engines = Self::build_engines(&client);
        Self {
            engines,
            app,
            fallback: None,
        }
    }

    fn build_engines(client: &Arc<reqwest::Client>) -> Vec<EngineEntry> {
        vec![
            EngineEntry {
                name: "duckduckgo",
                source: Box::new(DuckDuckGoSource::new(client.clone())),
            },
            EngineEntry {
                name: "brave",
                source: Box::new(BraveHtmlSource::new(client.clone())),
            },
            EngineEntry {
                name: "bing",
                source: Box::new(BingHtmlSource::new(client.clone())),
            },
            EngineEntry {
                name: "mojeek",
                source: Box::new(MojeekHtmlSource::new(client.clone())),
            },
            EngineEntry {
                name: "marginalia",
                source: Box::new(MarginaliaHtmlSource::new(client.clone())),
            },
            EngineEntry {
                name: "startpage",
                source: Box::new(StartpageHtmlSource::new(client.clone())),
            },
        ]
    }

    fn emit_health(&self, backend: &str, status: &str, result_count: usize, latency_ms: u64) {
        if let Some(app) = &self.app {
            let _ = app.emit(
                EV_META_SEARCH_HEALTH,
                MetaSearchHealthPayload {
                    backend: backend.to_string(),
                    status: status.to_string(),
                    result_count,
                    latency_ms,
                },
            );
        }
    }
}

fn dedup_key(url: &str) -> String {
    let trimmed = url.split('#').next().unwrap_or(url);
    trimmed.trim_end_matches('/').to_lowercase()
}

#[async_trait]
impl Source for MetaSearchSource {
    fn name(&self) -> &'static str {
        "meta_search"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let per_engine_limit = limit.max(8);
        let engine_budget = backend_timeout(per_engine_limit);
        let (tx, rx) = mpsc::channel::<anyhow::Result<Document>>(64);

        // Determine active engines after circuit-breaker check.
        let mut active_count = 0usize;
        for entry in &self.engines {
            let is_open = {
                let health = HEALTH.lock();
                health.get(entry.name).map(|h| h.is_open()).unwrap_or(false)
            };
            if is_open {
                self.emit_health(entry.name, "circuit_open", 0, 0);
                continue;
            }
            active_count += 1;

            let name = entry.name;
            let stream = entry
                .source
                .search(keywords.clone(), per_engine_limit)
                .await;
            let tx = tx.clone();
            let app_clone = self.app.clone();

            tokio::spawn(async move {
                let started = Instant::now();
                let mut count = 0usize;
                let mut saw_error = false;
                let mut saw_rate_limit = false;
                let mut timed_out = false;

                let stream = stream.take(per_engine_limit);
                tokio::pin!(stream);
                let result = tokio::time::timeout(engine_budget, async {
                    while let Some(item) = stream.next().await {
                        match &item {
                            Ok(_) => count += 1,
                            Err(e) => {
                                saw_error = true;
                                // A 429/rate-limit means "healthy but throttled",
                                // not "broken" — don't let it open the circuit.
                                if crate::events::classify_source_error(&e.to_string())
                                    == "rate_limit"
                                {
                                    saw_rate_limit = true;
                                }
                            }
                        }
                        if tx.send(item).await.is_err() {
                            break;
                        }
                    }
                })
                .await;

                if result.is_err() {
                    timed_out = true;
                    tracing::debug!("meta_search: engine {} timed out", name);
                }

                let latency_ms = started.elapsed().as_millis() as u64;

                // Update circuit breaker. Crucially, a healthy engine that simply
                // found nothing (HTTP 200, zero results) must NOT be treated as a
                // failure — otherwise a few niche/empty queries (or an engine
                // serving anti-bot challenge pages) open the circuit and silently
                // shrink web coverage. Only real transport/HTTP errors count.
                let (status_str, outcome) = if timed_out && count == 0 {
                    ("timeout", Outcome::Failure) // slow AND produced nothing -> fault
                } else if timed_out {
                    ("partial", Outcome::Neutral) // slow but returned results
                } else if count == 0 && saw_rate_limit {
                    ("throttled", Outcome::Neutral) // rate-limited, not broken
                } else if count == 0 && saw_error {
                    ("error", Outcome::Failure) // a real transport/HTTP error
                } else if count == 0 {
                    ("empty", Outcome::Neutral) // zero results -> not a fault
                } else {
                    ("ok", Outcome::Success)
                };

                {
                    let mut health = HEALTH.lock();
                    let entry = health.entry(name).or_default();
                    match outcome {
                        Outcome::Success => entry.record_success(),
                        Outcome::Failure => entry.record_failure(),
                        Outcome::Neutral => {}
                    }
                }

                // Emit health event.
                if let Some(app) = app_clone {
                    let _ = app.emit(
                        EV_META_SEARCH_HEALTH,
                        MetaSearchHealthPayload {
                            backend: name.to_string(),
                            status: status_str.to_string(),
                            result_count: count,
                            latency_ms,
                        },
                    );
                }
            });
        }

        // If all circuits are open, fall back to the public SearXNG pool —
        // unless this aggregator was built without one (it backs the in-process
        // local SearXNG server), in which case return an empty page so callers
        // move on instead of looping back into us.
        if active_count == 0 {
            if let Some(fallback) = &self.fallback {
                tracing::info!("meta_search: all circuits open, falling back to SearXNG pool");
                let fallback = Arc::clone(fallback);
                return fallback.search(keywords, limit).await;
            }
            tracing::info!("meta_search: all circuits open and no pool fallback — empty result");
            return stream::empty().boxed();
        }

        drop(tx);

        let seen: HashSet<String> = HashSet::new();
        stream::unfold((rx, seen), move |(mut rx, mut seen)| async move {
            while let Some(item) = rx.recv().await {
                match item {
                    Ok(mut doc) => {
                        let key = dedup_key(&doc.url);
                        if seen.insert(key) {
                            if !matches!(doc.source.as_str(), "meta_search") {
                                doc.source = format!("meta_search/{}", doc.source);
                            }
                            return Some((Ok(doc), (rx, seen)));
                        }
                        continue;
                    }
                    Err(e) => return Some((Err(e), (rx, seen))),
                }
            }
            None
        })
        .take(limit)
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_key_normalizes() {
        assert_eq!(
            dedup_key("https://Example.com/Path/"),
            dedup_key("https://example.com/path")
        );
        assert_eq!(
            dedup_key("https://example.com/page#section"),
            dedup_key("https://example.com/page")
        );
    }

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let mut h = BackendHealth::default();
        assert!(!h.is_open());
        for _ in 0..CIRCUIT_OPEN_FAILURES {
            h.record_failure();
        }
        assert!(h.is_open());
        h.record_success();
        assert!(!h.is_open());
    }

    #[test]
    fn below_threshold_failures_do_not_open() {
        // Only a real-failure streak opens a circuit; empties/throttles are
        // Neutral (no record_*), so an engine serving empty/challenge pages is
        // never suppressed.
        let mut h = BackendHealth::default();
        for _ in 0..CIRCUIT_OPEN_FAILURES - 1 {
            h.record_failure();
        }
        assert!(!h.is_open(), "stays closed below the failure threshold");
        h.record_success();
        h.record_failure();
        assert!(!h.is_open(), "a success resets the failure streak");
    }
}
