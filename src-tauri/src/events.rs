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

// Per-candidate event with ranking + reject reason. Augments EV_FOUND
// (kept for backward compat with the existing UI) by emitting one event
// per de-duplicated candidate after ranking, including those that won't
// be downloaded so the UI can show greyed "rejected" entries.
pub const EV_CANDIDATE: &str = "df:candidate";
pub const EV_RANKING_DONE: &str = "df:ranking_done";

// AI model lifecycle events (Tier 2 + Tier 3 + model manager).
pub const EV_MODEL_PROGRESS: &str = "df:model_progress";
pub const EV_MODEL_STATUS: &str = "df:model_status";

// Universal pipeline-stage event. Emitted at the boundary of every
// orchestrator phase so the UI can render an at-a-glance progress strip.
pub const EV_PIPELINE_STAGE: &str = "df:pipeline_stage";

// Meta-search backend health (emitted after each fan-out completes)
pub const EV_META_SEARCH_HEALTH: &str = "df:meta_search_health";

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
    /// Coarse classification so the frontend can dedup repeat errors of the
    /// same category (e.g. 8× "rate_limit" from Brave become a single row
    /// with a count badge). Values: "rate_limit" | "forbidden" |
    /// "server_error" | "timeout" | "parse_error" | "other".
    pub kind: String,
}

/// Map an error string to a coarse class for UI dedup. Cheap heuristics —
/// we don't need precision, just stable buckets.
pub fn classify_source_error(msg: &str) -> &'static str {
    let lower = msg.to_lowercase();
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("too many requests")
    {
        "rate_limit"
    } else if lower.contains("403")
        || lower.contains("401")
        || lower.contains("forbidden")
        || lower.contains("unauthorized")
        || lower.contains("blocked")
    {
        "forbidden"
    } else if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("server error")
    {
        "server_error"
    } else if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("operation took")
    {
        "timeout"
    } else if lower.contains("parse")
        || lower.contains("regex")
        || lower.contains("decode")
        || lower.contains("non-json")
        || lower.contains("missing")
    {
        "parse_error"
    } else {
        "other"
    }
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
pub struct ErrorPayload {
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidatePayload {
    #[serde(flatten)]
    pub doc: Document,
    /// Sources that returned this candidate (>=1; many for cross-source dupes).
    pub sources: Vec<String>,
    pub tfidf: f32,
    pub rrf: f32,
    pub authority: f32,
    pub score: f32,
    /// "kept" | "rejected" | "borderline"
    pub status: String,
    pub reject_reason: Option<String>,
    /// 1-indexed final rank within the kept set, or None if rejected.
    pub final_rank: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankingDonePayload {
    pub total_candidates: usize,
    pub kept: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelProgressPayload {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub bytes_per_sec: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatusPayload {
    pub model_id: String,
    /// One of: "downloading", "verifying", "ready", "failed", "cancelled",
    /// "embedding", "llm_warming", "llm_expanding", "llm_filtering".
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineStagePayload {
    /// One of: "discovery", "rank", "semantic_rerank", "llm_expand",
    /// "llm_filter", "citation_enrich", "download", "extract".
    pub stage: String,
    /// "started" | "progress" | "done" | "skipped"
    pub state: String,
    /// Optional progress numerator (e.g. files extracted so far).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Optional progress denominator (e.g. total candidates to filter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Free-form detail for the UI (e.g. "12 sources active").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetaSearchHealthPayload {
    pub backend: String,
    /// "ok" | "timeout" | "circuit_open" | "error"
    pub status: String,
    pub result_count: usize,
    pub latency_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::classify_source_error;

    #[test]
    fn classifies_rate_limit_bucket() {
        assert_eq!(classify_source_error("429 Too Many Requests"), "rate_limit");
        assert_eq!(classify_source_error("rate limit exceeded"), "rate_limit");
    }

    #[test]
    fn classifies_forbidden_bucket() {
        assert_eq!(classify_source_error("HTTP 403 forbidden"), "forbidden");
        assert_eq!(classify_source_error("blocked by upstream"), "forbidden");
        assert_eq!(classify_source_error("401 Unauthorized"), "forbidden");
    }

    #[test]
    fn classifies_server_error_bucket() {
        assert_eq!(
            classify_source_error("HTTP 502 Bad Gateway"),
            "server_error"
        );
        assert_eq!(
            classify_source_error("upstream returned 503"),
            "server_error"
        );
        assert_eq!(
            classify_source_error("internal server error"),
            "server_error"
        );
    }

    #[test]
    fn classifies_timeout_bucket() {
        assert_eq!(classify_source_error("connection timed out"), "timeout");
        assert_eq!(classify_source_error("operation took 30s"), "timeout");
    }

    #[test]
    fn classifies_parse_error_bucket() {
        assert_eq!(classify_source_error("failed to parse JSON"), "parse_error");
        assert_eq!(
            classify_source_error("missing field 'title'"),
            "parse_error"
        );
    }

    #[test]
    fn falls_through_to_other() {
        assert_eq!(classify_source_error("some weird unknown failure"), "other");
    }
}
