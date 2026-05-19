//! In-process SearXNG-compatible HTTP server.
//!
//! Exposes `GET /search?q=...&format=json` on `127.0.0.1:<random-port>`
//! returning the exact JSON shape a real SearXNG instance emits, backed by
//! the existing `MetaSearchSource` aggregator. This satisfies the
//! "local SearXNG without Docker" requirement: the Tauri app starts the
//! server on launch, so any code path (including `SearxngPoolSource`) can
//! point at `http://127.0.0.1:<port>/search` instead of needing a real
//! SearXNG container or a public instance.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use futures::stream::StreamExt;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

use super::Source;

/// Set once by `spawn_server` on app startup; read by `SearxngPoolSource` to
/// route its first query at the in-process server before falling back to
/// public SearXNG instances.
static LOCAL_PORT: OnceCell<u16> = OnceCell::new();

/// The bound port of the in-process SearXNG-compatible server, if it has been
/// started. `None` in unit tests or before `spawn_server` is called.
pub fn local_port() -> Option<u16> {
    LOCAL_PORT.get().copied()
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub pageno: Option<u32>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub engine: String,
    pub score: f32,
    pub category: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub number_of_results: usize,
    pub results: Vec<SearchResult>,
    pub answers: Vec<serde_json::Value>,
    pub corrections: Vec<serde_json::Value>,
    pub infoboxes: Vec<serde_json::Value>,
    pub suggestions: Vec<serde_json::Value>,
    pub unresponsive_engines: Vec<serde_json::Value>,
}

#[derive(Clone)]
struct AppCtx {
    backend: Arc<dyn Source>,
}

/// SearXNG mirrors `?q=` straight through; honor that.
async fn search(
    State(ctx): State<AppCtx>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let keywords: Vec<String> = params.q.split_whitespace().map(|s| s.to_string()).collect();
    if keywords.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(SearchResponse {
                query: params.q,
                number_of_results: 0,
                results: vec![],
                answers: vec![],
                corrections: vec![],
                infoboxes: vec![],
                suggestions: vec![],
                unresponsive_engines: vec![],
            }),
        );
    }

    let stream = ctx.backend.search(keywords, 30).await;
    tokio::pin!(stream);

    let mut results: Vec<SearchResult> = Vec::new();
    let mut rank = 0usize;
    while let Some(item) = stream.next().await {
        if let Ok(doc) = item {
            rank += 1;
            // SearXNG-style scores decay with rank; rough emulation.
            let score = 1.0 / (rank as f32);
            results.push(SearchResult {
                url: doc.url,
                title: doc.title,
                content: doc.abstract_,
                engine: doc.source,
                score,
                category: "general",
            });
        }
    }

    (
        StatusCode::OK,
        Json(SearchResponse {
            query: params.q,
            number_of_results: results.len(),
            results,
            answers: vec![],
            corrections: vec![],
            infoboxes: vec![],
            suggestions: vec![],
            unresponsive_engines: vec![],
        }),
    )
}

async fn healthz() -> &'static str {
    "ok"
}

/// Bind to `127.0.0.1` on an OS-assigned port, spawn the Axum server, and
/// return the bound port. The server runs for the lifetime of the process.
/// Safe to call multiple times; only the first call's port is registered.
pub async fn spawn_server(backend: Arc<dyn Source>) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let ctx = AppCtx { backend };
    let router = Router::new()
        .route("/search", get(search))
        .route("/healthz", get(healthz))
        .with_state(ctx);

    let _ = LOCAL_PORT.set(port);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("local_searxng server exited: {}", e);
        }
    });

    // Health-poll briefly so callers know we're ready.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("reqwest client");
    let url = format!("http://127.0.0.1:{port}/healthz");
    for _ in 0..20 {
        if client.get(&url).send().await.is_ok() {
            tracing::info!("local_searxng ready on 127.0.0.1:{port}");
            return Ok(port);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Server didn't respond in 2s — return the port anyway; queries will
    // surface errors if it is genuinely wedged.
    tracing::warn!("local_searxng health check timed out; returning port {port} anyway");
    Ok(port)
}
