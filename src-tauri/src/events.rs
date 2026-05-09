//! Event payload types emitted to the frontend via `app.emit("<name>", payload)`.
//!
//! Event payloads are used to synchronize state between the Rust backend and the React frontend.

use serde::Serialize;

use crate::sources::Document;

pub const EV_KEYWORDS: &str = "df:keywords";
pub const EV_SUBQUERY_START: &str = "df:subquery_start";
pub const EV_SOURCE_START: &str = "df:source_start";
pub const EV_SOURCE_DONE: &str = "df:source_done";
pub const EV_SOURCE_ERROR: &str = "df:source_error";
pub const EV_FOUND: &str = "df:found";
pub const EV_FOUND_TOTAL: &str = "df:found_total";
pub const EV_DOWNLOAD_STARTED: &str = "df:download_started";
pub const EV_DOWNLOAD_PROGRESS: &str = "df:download_progress";
pub const EV_DOWNLOAD_DONE: &str = "df:download_done";
pub const EV_DOWNLOAD_FAILED: &str = "df:download_failed";
pub const EV_CANCELLED: &str = "df:cancelled";
pub const EV_COMPLETE: &str = "df:complete";
pub const EV_ERROR: &str = "df:error";
pub const EV_FILTERED: &str = "df:filtered";

// SearXNG setup streaming events
pub const EV_SEARXNG_LOG: &str = "df:searxng_setup_log";
pub const EV_SEARXNG_STAGE: &str = "df:searxng_setup_stage";

#[derive(Debug, Clone, Serialize)]
pub struct KeywordsPayload {
    pub query: String,
    pub sub_queries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubQueryStartPayload {
    pub sub_query: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStartPayload {
    pub source: String,
    pub sub_query: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceDonePayload {
    pub source: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceErrorPayload {
    pub source: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FoundPayload {
    pub title: String,
    pub source: String,
    pub url: String,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FoundTotalPayload {
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadStartedPayload {
    pub url: String,
    pub title: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgressPayload {
    pub url: String,
    pub title: String,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadDonePayload {
    #[serde(flatten)]
    pub doc: Document,
    pub local_path: String,
    pub absolute_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_path: Option<String>,
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadFailedPayload {
    #[serde(flatten)]
    pub doc: Document,
    pub error: String,
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletePayload {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
    pub folder: String,
    pub manifest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilteredPayload {
    pub source: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorPayload {
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearxngLogPayload {
    /// "stdout" | "stderr" | "info"
    pub stream: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearxngStagePayload {
    /// One of: "checking_docker", "checking_port", "pulling", "starting",
    /// "waiting_health", "ok", "failed".
    pub stage: String,
    pub detail: Option<String>,
}
