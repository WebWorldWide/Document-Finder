//! Built-in meta-search aggregator — the zero-config replacement for SearXNG.
//!
//! Fans out the query to all six HTML web scrapers concurrently
//! (DuckDuckGo, Brave, Bing, Mojeek, Marginalia, Startpage), collects their
//! result streams into a single unified stream, and dedupes by lowercased
//! URL so cross-engine duplicates don't double-emit.
//!
//! This source is the default web backend so a freshly installed app can
//! search without any external service running. Heavyweight dedup (by DOI /
//! normalized title / author+year fingerprint) still happens downstream in
//! `engine::dedup`; this layer only does cheap URL-level dedup to keep the
//! stream small and avoid burning rank slots.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::bing_html::BingHtmlSource;
use super::brave_html::BraveHtmlSource;
use super::duckduckgo::DuckDuckGoSource;
use super::marginalia_html::MarginaliaHtmlSource;
use super::mojeek_html::MojeekHtmlSource;
use super::startpage_html::StartpageHtmlSource;
use super::{Document, Source};

/// Per-engine ceiling on how long we'll wait before giving up on its stream.
/// Slow engines shouldn't hold the aggregate stream hostage.
const PER_ENGINE_TIMEOUT: Duration = Duration::from_secs(12);

pub struct MetaSearchSource {
    engines: Vec<Box<dyn Source>>,
}

impl MetaSearchSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        let engines: Vec<Box<dyn Source>> = vec![
            Box::new(DuckDuckGoSource::new(client.clone())),
            Box::new(BraveHtmlSource::new(client.clone())),
            Box::new(BingHtmlSource::new(client.clone())),
            Box::new(MojeekHtmlSource::new(client.clone())),
            Box::new(MarginaliaHtmlSource::new(client.clone())),
            Box::new(StartpageHtmlSource::new(client)),
        ];
        Self { engines }
    }
}

fn dedup_key(url: &str) -> String {
    // Lowercased URL with trailing slash and fragment stripped. Cheap; the
    // engine layer in dedup.rs does the smart matching later.
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
        // Each engine still does its own internal pagination; we ask each for
        // up to `limit` results and the merged stream takes the first `limit`
        // unique URLs.
        let per_engine_limit = limit.max(8);

        // mpsc channel collects results from every engine concurrently.
        let (tx, rx) = mpsc::channel::<anyhow::Result<Document>>(64);

        for engine in &self.engines {
            let name = engine.name();
            let stream = engine.search(keywords.clone(), per_engine_limit).await;
            let tx = tx.clone();
            tokio::spawn(async move {
                let stream = stream.take(per_engine_limit);
                tokio::pin!(stream);
                let _ = tokio::time::timeout(PER_ENGINE_TIMEOUT, async {
                    while let Some(item) = stream.next().await {
                        // If the receiver hung up, stop pumping.
                        if tx.send(item).await.is_err() {
                            break;
                        }
                    }
                })
                .await
                .map_err(|_| {
                    tracing::debug!("meta_search: engine {} timed out", name);
                });
            });
        }
        // Drop our own producer end so the receive stream finishes once all
        // spawned senders go out of scope.
        drop(tx);

        let seen: HashSet<String> = HashSet::new();
        stream::unfold((rx, seen), move |(mut rx, mut seen)| async move {
            while let Some(item) = rx.recv().await {
                match item {
                    Ok(mut doc) => {
                        let key = dedup_key(&doc.url);
                        if seen.insert(key) {
                            // Tag the unified source so the UI knows where it
                            // came from at a glance.
                            if !matches!(doc.source.as_str(), "meta_search") {
                                doc.source = format!("meta_search/{}", doc.source);
                            }
                            return Some((Ok(doc), (rx, seen)));
                        }
                        // Duplicate URL — skip and pull next.
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
            dedup_key("https://example.com/x#frag"),
            dedup_key("https://example.com/x")
        );
    }
}
