//! End-to-end test for the in-process SearXNG-compatible HTTP server.
//!
//! Boots `local_searxng::spawn_server` against a deterministic stub Source
//! on a random localhost port, queries `/healthz` and `/search?format=json`,
//! and asserts the response matches the JSON shape a real SearXNG instance
//! emits. The stub avoids hitting the network and avoids loading Tauri's
//! runtime DLLs (which can fail at link time on Windows test binaries).

use async_trait::async_trait;
use document_finder_lib::sources::{local_searxng, Document, Source};
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

struct StubSource {
    docs: Vec<Document>,
}

#[async_trait]
impl Source for StubSource {
    fn name(&self) -> &'static str {
        "stub"
    }

    async fn search(
        &self,
        _keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let docs: Vec<Document> = self.docs.iter().take(limit).cloned().collect();
        stream::iter(docs.into_iter().map(Ok)).boxed()
    }
}

fn stub() -> Arc<dyn Source> {
    Arc::new(StubSource {
        docs: vec![
            Document {
                title: "Rust Programming Language".to_string(),
                url: "https://www.rust-lang.org".to_string(),
                source: "stub".to_string(),
                authors: vec![],
                year: None,
                abstract_: Some("Safe systems programming".to_string()),
                identifier: None,
            },
            Document {
                title: "The Rust Book".to_string(),
                url: "https://doc.rust-lang.org/book/".to_string(),
                source: "stub".to_string(),
                authors: vec![],
                year: None,
                abstract_: None,
                identifier: None,
            },
        ],
    })
}

#[derive(Debug, Deserialize)]
struct SearxLikeResponse {
    query: String,
    number_of_results: usize,
    results: Vec<SearxLikeResult>,
    answers: Vec<serde_json::Value>,
    suggestions: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SearxLikeResult {
    url: String,
    title: String,
    content: Option<String>,
    engine: String,
    score: f32,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_starts_and_serves_searxng_shape() {
    let port = local_searxng::spawn_server(stub())
        .await
        .expect("spawn_server should bind to a localhost port");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("reqwest client");

    let health = http
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await
        .expect("healthz reachable");
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");

    let search = http
        .get(format!("http://127.0.0.1:{port}/search"))
        .query(&[("q", "rust book"), ("format", "json")])
        .send()
        .await
        .expect("search reachable");
    assert_eq!(search.status(), 200);

    let body: SearxLikeResponse = search
        .json()
        .await
        .expect("response is SearXNG-shaped JSON");
    assert_eq!(body.query, "rust book");
    assert_eq!(body.number_of_results, 2);
    assert_eq!(body.results.len(), 2);
    assert_eq!(body.results[0].url, "https://www.rust-lang.org");
    assert!(body.answers.is_empty());
    assert!(body.suggestions.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_query_returns_400_with_empty_results() {
    let port = local_searxng::spawn_server(stub()).await.unwrap();

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let resp = http
        .get(format!("http://127.0.0.1:{port}/search"))
        .query(&[("q", "   "), ("format", "json")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: SearxLikeResponse = resp.json().await.unwrap();
    assert_eq!(body.number_of_results, 0);
    assert!(body.results.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_port_registered_after_spawn() {
    // Production calls spawn_server exactly once; tests race against each
    // other but each succeeds individually. `local_port()` must be set after
    // any spawn — its exact value depends on which test won the OnceCell race.
    let _port = local_searxng::spawn_server(stub()).await.unwrap();
    assert!(
        local_searxng::local_port().is_some(),
        "local_port should be registered after spawn_server completes",
    );
}
