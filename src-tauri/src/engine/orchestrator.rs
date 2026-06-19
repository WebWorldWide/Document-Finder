//! Core pipeline management for discovering, downloading, and bundling research documents.

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::citation_graph::enrich_with_citation_graph;
use super::db::DbManager;
use super::dedup::{AddOutcome, Deduplicator};
use super::downloader::{download, DownloadOutcome};
use super::extract::extract_text;
use super::query::{expand_query_broad, parse_query, safe_folder};
use super::ranking::{flag_rejects, rank_candidates, RankedDoc};
use super::runlog;
use crate::events::*;
use crate::sources::{
    build_source, make_client, make_download_client, Document, SourceOptions, SOURCE_IDS,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunRequest {
    pub query: String,
    pub sources: Vec<String>,
    pub out_dir: String,
    #[serde(default = "default_per_source")]
    pub per_source: usize,
    #[serde(default = "default_max_total")]
    pub max_total: usize,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_extract")]
    pub extract: bool,
    /// When true, after Tier 1 ranking we enrich the top candidates with
    /// Semantic Scholar citation/reference graph data and boost candidates
    /// that are connected to other top candidates. Adds API latency.
    #[serde(default)]
    pub use_citation_graph: bool,
    /// When true, after Tier 1 ranking we re-embed the top 100 candidates
    /// with the bge-small-en-v1.5 model and blend cosine similarity into
    /// the final score. Defaults to true; auto-falls-back to Tier 1 only
    /// if the embedding model fails to load.
    #[serde(default = "default_use_semantic_rerank")]
    pub use_semantic_rerank: bool,
    /// When true, the local LLM (Tier 3) generates additional sub-queries
    /// from the user's input before discovery starts. Currently a no-op
    /// when the LLM model isn't loaded.
    #[serde(default = "default_use_llm_expansion")]
    pub use_llm_expansion: bool,
    /// When true, the local LLM (Tier 3) judges borderline candidates
    /// (50–70th percentile of post-rerank score) for topical fit.
    /// Currently a no-op when the LLM model isn't loaded.
    #[serde(default = "default_use_llm_filter")]
    pub use_llm_filter: bool,
    /// LLM model id from the registry. If None, uses the registry default.
    #[serde(default)]
    pub llm_model_id: Option<String>,
    /// When true, before downloading a candidate the pipeline checks SIBLING
    /// libraries (other per-query folders under the same root) for an identical
    /// already-downloaded file and copies it in instead of re-fetching. Opt-in;
    /// saves bandwidth across overlapping searches while keeping each library
    /// self-contained.
    #[serde(default)]
    pub cross_run_reuse: bool,
    #[serde(default)]
    pub source_options: HashMap<String, SourceOptions>,
}

fn default_use_semantic_rerank() -> bool {
    true
}
fn default_use_llm_expansion() -> bool {
    true
}
fn default_use_llm_filter() -> bool {
    true
}

fn default_per_source() -> usize {
    100
}
fn default_max_total() -> usize {
    500
}
fn default_concurrency() -> usize {
    8
}
fn default_extract() -> bool {
    true
}

/// Max concurrent in-flight requests allowed against a single source, across
/// ALL sub-queries in a discovery wave. With broad LLM expansion a wave can
/// carry ~24 sub-queries; without this cap that would fire ~24 simultaneous
/// requests at every source, instantly tripping rate limits (429), circuit
/// breakers, and shared-pool limits — making results *worse*, not broader.
/// Different sources still run fully in parallel with each other; this only
/// serializes the sub-queries hitting the *same* source into small batches.
fn source_concurrency(name: &str) -> usize {
    match name {
        // Each internally fans out to several engines, or rate-limits hard.
        "meta_search" | "searxng" | "web" => 2,
        // Semantic Scholar's public API 429s aggressively and shares a global
        // rate pool; keep it gentle so it doesn't lock us out for the session.
        "semantic_scholar" => 2,
        // Zenodo's guest API rate-limits (~60/min) — keep it gentle.
        "zenodo" => 2,
        // Structured APIs tolerate a handful of concurrent requests fine.
        _ => 4,
    }
}

/// Wave deadlines derived from the run's discovery cap (`max_total`). These are
/// the *worst-case* caps that force-stop a wave when a source hangs — a wave
/// normally ends far sooner, either when all source tasks drain naturally or
/// when `discover_wave`'s early-exit fires once `max_total` unique docs are in
/// (see the `tokio::select!` there). The deadlines were cut to an interactive
/// range so a slow/rate-limited source can no longer hold the whole run open
/// for minutes; a user cancel still races the deadline and wins within ~1s.
///
///   max_total   75 (Light)       → 16s / 13s
///   max_total  500 (Balanced)    → 27s / 22s
///   max_total 1500 (Deep)        → 52s / 42s
///   max_total 4000+ (Exhaustive) → 60s / 45s (clamped)
fn wave_deadlines(max_total: usize) -> (Duration, Duration) {
    let w1 = (15 + max_total / 40).clamp(15, 60) as u64;
    let w2 = (12 + max_total / 50).clamp(12, 45) as u64;
    (Duration::from_secs(w1), Duration::from_secs(w2))
}

/// Delete leftover `*.part` download temps in the run folder.
async fn sweep_stale_parts(folder: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(folder).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("part") {
            let _ = tokio::fs::remove_file(&p).await;
        }
    }
}

/// Emit a stage transition. State strings: "started" | "progress" | "done"
/// | "skipped". Helper so callsites stay tidy.
fn emit_stage(
    app: &AppHandle,
    stage: &str,
    state: &str,
    count: Option<u64>,
    total: Option<u64>,
    message: Option<String>,
) {
    let _ = app.emit(
        EV_PIPELINE_STAGE,
        PipelineStagePayload {
            stage: stage.to_string(),
            state: state.to_string(),
            count,
            total,
            message,
        },
    );
}

/// Fan out one discovery wave: spawn a task per (sub_query, source), drain them
/// (bounded by `deadline`, racing a user cancel) into the shared `dedup`, and
/// extend `all_keywords` for ranking. Emits EV_SUBQUERY_START / EV_SOURCE_START
/// / EV_FOUND / EV_SOURCE_DONE as work happens. Returns true if it stopped early
/// (cancel or deadline) instead of draining naturally.
#[allow(clippy::too_many_arguments)]
async fn discover_wave(
    app: &AppHandle,
    client: &Arc<reqwest::Client>,
    cancel: &CancellationToken,
    dedup: &Arc<AsyncMutex<Deduplicator>>,
    all_keywords: &mut Vec<String>,
    sub_queries: &[String],
    effective_sources: &[String],
    req: &RunRequest,
    deadline: std::time::Duration,
    source_sems: &HashMap<String, Arc<Semaphore>>,
) -> bool {
    let mut tasks: JoinSet<()> = JoinSet::new();

    for sub in sub_queries {
        let keywords = parse_query(sub);
        let keywords = if keywords.is_empty() {
            sub.split_whitespace().map(String::from).collect::<Vec<_>>()
        } else {
            keywords
        };
        if keywords.is_empty() {
            continue;
        }
        all_keywords.extend(keywords.iter().cloned());
        let _ = app.emit(
            EV_SUBQUERY_START,
            SubQueryStartPayload {
                sub_query: sub.clone(),
                keywords: keywords.clone(),
            },
        );

        for sname in effective_sources {
            if !SOURCE_IDS.contains(&sname.as_str()) {
                let _ = app.emit(
                    EV_SOURCE_ERROR,
                    SourceErrorPayload {
                        source: sname.clone(),
                        error: "unknown source".into(),
                        kind: "other".into(),
                    },
                );
                continue;
            }
            let opts = req.source_options.get(sname).cloned().unwrap_or_default();
            let Some(src) = build_source(sname, opts, Arc::clone(client), Some(app.clone())) else {
                continue;
            };

            let app_t = app.clone();
            let cancel_t = cancel.clone();
            let dedup_t = Arc::clone(dedup);
            let sub_t = sub.clone();
            let sname_t = sname.clone();
            let keywords_t = keywords.clone();
            let per_source = req.per_source;
            let max_total = req.max_total;
            // Per-source throttle: this task holds one permit for the lifetime of
            // its source stream, bounding concurrent requests to this source
            // regardless of how many sub-queries the wave carries.
            let sem = source_sems.get(sname).cloned();

            tasks.spawn(async move {
                // Acquire the source permit before opening the stream. Released
                // when `_permit` drops at the end of the task. If the semaphore
                // is somehow missing/closed, fall through unthrottled rather than
                // dropping the sub-query.
                let _permit = match sem {
                    Some(s) => s.acquire_owned().await.ok(),
                    None => None,
                };
                // A cancel that arrived while we were queued on the permit: bail
                // before spending a request.
                if cancel_t.is_cancelled() {
                    return;
                }
                let _ = app_t.emit(
                    EV_SOURCE_START,
                    SourceStartPayload {
                        source: sname_t.clone(),
                        sub_query: sub_t.clone(),
                    },
                );

                let mut count = 0usize;
                let mut rank_in_source = 0usize;
                // Per-task dedup so a source spamming one error class (e.g. 8×
                // "429" from Brave during a multi-page scrape) collapses into a
                // single UI surface.
                let mut seen_kinds: std::collections::HashSet<&'static str> =
                    std::collections::HashSet::new();
                let mut stream = src.search(keywords_t.clone(), per_source).await;

                while let Some(item) = stream.next().await {
                    if cancel_t.is_cancelled() {
                        break;
                    }
                    match item {
                        Ok(doc) => {
                            rank_in_source += 1;
                            // Hard cap (cumulative across waves via the shared
                            // dedup) so a slow source can't push past max_total.
                            let total_now = {
                                let mut d = dedup_t.lock().await;
                                if d.len() >= max_total {
                                    break;
                                }
                                let title_for_evt = doc.title.clone();
                                let url_for_evt = doc.url.clone();
                                let source_for_evt = doc.source.clone();
                                let outcome = d.add(doc, &sname_t, rank_in_source);
                                let total = d.len();
                                drop(d);
                                // Only count + announce a doc that's actually NEW
                                // to the dedup. A merged duplicate doesn't change
                                // the unique total, so counting it would inflate
                                // this source's live contribution and re-fire
                                // EV_FOUND with an unchanged total.
                                if matches!(outcome, AddOutcome::New(_)) {
                                    count += 1;
                                    let _ = app_t.emit(
                                        EV_FOUND,
                                        FoundPayload {
                                            title: title_for_evt,
                                            source: source_for_evt,
                                            url: url_for_evt,
                                            total,
                                        },
                                    );
                                }
                                total
                            };
                            if total_now >= max_total {
                                break;
                            }
                        }
                        Err(e) => {
                            let err_str = e.to_string();
                            runlog::log(runlog::Event::SourceError {
                                source: &sname_t,
                                error: &err_str,
                                sub_query: Some(&sub_t),
                            });
                            let kind = crate::events::classify_source_error(&err_str);
                            // Parse-class errors from the HTML *scrapers* mean
                            // markup drift — users can't act on them, so stay
                            // silent. But for structured-API sources (arxiv,
                            // openalex, semantic_scholar, internet_archive, doaj,
                            // gutenberg) a "parse" failure is a real, reportable
                            // outage (API shape changed / returned an error body),
                            // so surface it instead of swallowing it.
                            let is_web_scraper = matches!(
                                sname_t.as_str(),
                                "web"
                                    | "brave"
                                    | "bing"
                                    | "mojeek"
                                    | "marginalia"
                                    | "startpage"
                                    | "meta_search"
                                    | "searxng"
                            );
                            if kind == "parse_error" && is_web_scraper {
                                continue;
                            }
                            if !seen_kinds.insert(kind) {
                                continue;
                            }
                            let _ = app_t.emit(
                                EV_SOURCE_ERROR,
                                SourceErrorPayload {
                                    source: sname_t.clone(),
                                    error: err_str,
                                    kind: kind.into(),
                                },
                            );
                        }
                    }
                }
                let _ = app_t.emit(
                    EV_SOURCE_DONE,
                    SourceDonePayload {
                        source: sname_t.clone(),
                        count,
                    },
                );
            });
        }
    }

    // Drain the spawned tasks, but cap total wave time so one slow / rate-
    // limited source can't stall the run, and react to a user cancel promptly.
    // Each task only checks the cancel token *between* stream items, so racing
    // the drain against `cancel.cancelled()` makes Stop take effect within ~1s.
    let stop_early = tokio::select! {
        biased;
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(deadline) => true,
        // Early-exit: the moment the shared dedup already holds `max_total`
        // unique docs, stop the wave instead of idling to the deadline while
        // slow/rate-limited sources keep streaming results we'd only discard.
        // Cheap poll; the hard cap inside each task also breaks its stream.
        _ = async {
            loop {
                tokio::time::sleep(Duration::from_millis(150)).await;
                if dedup.lock().await.len() >= req.max_total {
                    break;
                }
            }
        } => true,
        _ = async {
            while let Some(res) = tasks.join_next().await {
                if let Err(e) = res {
                    tracing::warn!("discovery task join error: {e}");
                }
            }
        } => false,
    };
    if stop_early {
        if cancel.is_cancelled() {
            tracing::info!("discovery cancelled by user — aborting in-flight tasks");
        } else {
            tracing::info!(
                "discovery wave stopped early (cap reached or deadline) — proceeding with partial results"
            );
        }
        tasks.shutdown().await;
    }
    stop_early
}

/// Run the optional LLM query-expansion (Balanced & Thorough both enable it via
/// `use_llm_expansion`) and return the extra sub-queries it produced. A request
/// that asks for semantic rerank but NOT expansion falls back to a lightweight
/// spell-fix. Built to run CONCURRENTLY with the first discovery wave so a cold
/// model load never blocks first results. No-op (and a "skipped" stage) when the
/// LLM feature is off or no model is on disk.
#[cfg(not(feature = "ai-llm"))]
async fn compute_llm_extras(
    app: &AppHandle,
    req: &RunRequest,
    cancel: &CancellationToken,
) -> Vec<String> {
    let _ = (req, cancel);
    emit_stage(
        app,
        "llm_expand",
        "skipped",
        None,
        None,
        Some("LLM feature disabled at build time".into()),
    );
    Vec::new()
}

#[cfg(feature = "ai-llm")]
async fn compute_llm_extras(
    app: &AppHandle,
    req: &RunRequest,
    cancel: &CancellationToken,
) -> Vec<String> {
    if cancel.is_cancelled() {
        emit_stage(app, "llm_expand", "skipped", None, None, None);
        return Vec::new();
    }

    if req.use_llm_expansion {
        // Tier 3a: LLM query expansion.
        emit_stage(app, "llm_expand", "started", None, None, None);
        let Some(llm) = ensure_llm_loaded(app).await else {
            emit_stage(
                app,
                "llm_expand",
                "skipped",
                None,
                None,
                Some("LLM model not downloaded".into()),
            );
            return Vec::new();
        };
        let _ = app.emit(
            crate::events::EV_MODEL_STATUS,
            crate::events::ModelStatusPayload {
                model_id: req.llm_model_id.clone().unwrap_or_else(|| "llm".into()),
                status: "llm_expanding".to_string(),
                detail: None,
            },
        );
        let prompt = crate::ai::llm::expansion_prompt(&req.query);
        let original = req.query.clone();
        // Race the (blocking) generation against cancel + a hard timeout so Stop
        // stays responsive and a wedged model can't hang the run. We pass a CHILD
        // of the run cancel token into generate(): a parent Stop still propagates
        // (cancelling the decode), and on the timeout branch we cancel the child
        // ourselves so the detached decode bails at its next per-token check and
        // releases the model mutex — instead of running to the full token budget
        // and blocking the next LLM stage (the filter) on the held lock. Cancelling
        // the child leaves the run-level token untouched so the pipeline proceeds.
        // Breadth scales the LLM fan-out: deeper runs ask for more sub-queries and
        // a proportionally larger token budget. Balanced keeps the historical
        // ~24 lines / 384 tokens. The per-token cancel check + the timeout below
        // keep a slow CPU decode from stalling the run regardless of the budget.
        let max_expansions = if req.max_total >= 2000 {
            40
        } else if req.max_total >= 1000 {
            32
        } else {
            24
        };
        let tok_budget = (max_expansions * 14 + 96).clamp(384, 700);
        let gen_cancel = cancel.child_token();
        let cancel_gen = gen_cancel.clone();
        let handle = tokio::task::spawn_blocking(move || {
            llm.blocking_lock()
                .generate(&prompt, tok_budget, &cancel_gen)
        });
        let text = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                emit_stage(app, "llm_expand", "skipped", None, None, None);
                return Vec::new();
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                gen_cancel.cancel(); // free the model lock from the runaway decode
                emit_stage(app, "llm_expand", "done", None, None, Some("expansion timed out".into()));
                return Vec::new();
            }
            r = handle => match r {
                Ok(Ok(t)) => t,
                _ => {
                    emit_stage(app, "llm_expand", "done", None, None, None);
                    return Vec::new();
                }
            }
        };
        let mut extras = crate::ai::llm::parse_expansion(&text, &original);
        // Enforce the depth-scaled cap (parse_expansion only applies the hard
        // ceiling); Balanced keeps 24, deeper runs allow more.
        extras.truncate(max_expansions);
        let n = extras.len() as u64;
        if n > 0 {
            tracing::info!("LLM expanded query into {} extras", n);
        }
        emit_stage(
            app,
            "llm_expand",
            "done",
            Some(n),
            None,
            if n > 0 {
                Some(format!("+{} sub-queries", n))
            } else {
                None
            },
        );
        extras
    } else if req.use_semantic_rerank {
        // Fallback for a custom request that wants rerank but explicitly NOT
        // expansion: lightweight spell correction only (the expansion prompt
        // already corrects spelling). The standard Balanced/Thorough presets
        // both enable expansion and take the branch above; this keeps a sane
        // behavior for anyone hand-building a RunRequest. No-op without a model.
        emit_stage(
            app,
            "llm_expand",
            "started",
            None,
            None,
            Some("spell-check".into()),
        );
        let Some(llm) = ensure_llm_loaded(app).await else {
            emit_stage(app, "llm_expand", "skipped", None, None, None);
            return Vec::new();
        };
        let prompt = crate::ai::llm::spellfix_prompt(&req.query);
        // Same child-token pattern as the expansion branch: cancel only the decode
        // on timeout so it releases the model mutex without aborting the run.
        let gen_cancel = cancel.child_token();
        let cancel_gen = gen_cancel.clone();
        let handle = tokio::task::spawn_blocking(move || {
            llm.blocking_lock().generate(&prompt, 32, &cancel_gen)
        });
        let text = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                emit_stage(app, "llm_expand", "skipped", None, None, None);
                return Vec::new();
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(20)) => {
                gen_cancel.cancel(); // free the model lock from the runaway decode
                emit_stage(app, "llm_expand", "done", None, None, None);
                return Vec::new();
            }
            r = handle => match r {
                Ok(Ok(t)) => t,
                _ => {
                    emit_stage(app, "llm_expand", "done", None, None, None);
                    return Vec::new();
                }
            }
        };
        emit_stage(app, "llm_expand", "done", None, None, None);
        if let Some(fixed) = crate::ai::llm::parse_spellfix(&text, &req.query) {
            tracing::info!("spell-fix: {:?} -> {:?}", req.query, fixed);
            return vec![fixed];
        }
        Vec::new()
    } else {
        emit_stage(app, "llm_expand", "skipped", None, None, None);
        Vec::new()
    }
}

/// Parallel download + parallel extraction over a fixed candidate list, writing
/// into `folder` (with text under `text_dir`) and indexing rows under `run_id`
/// in `db_path`. Shared by the full pipeline (`run_pipeline`) and the
/// retry-failed path (`run_retry_pipeline`) so both go through exactly one,
/// verified, download implementation. Returns `(done, failed)`. The caller emits
/// the surrounding stage + completion events.
#[allow(clippy::too_many_arguments)]
async fn download_and_extract(
    app: &AppHandle,
    candidates: Vec<Document>,
    folder: &Path,
    text_dir: &Path,
    db_path: &Path,
    run_id: i64,
    concurrency: usize,
    extract: bool,
    // When `Some(root)`, try reusing an identical file from a sibling library
    // under `root` before downloading (cross-run reuse; opt-in).
    reuse_root: Option<&Path>,
    cancel: &CancellationToken,
) -> (usize, usize) {
    // Two separate semaphores so download (network-bound) and extract
    // (CPU-bound) don't serialize each other: the download permit drops the
    // moment bytes are on disk, and a smaller extract permit (CPU count) gates
    // the heavy text parsing.
    let total = candidates.len();
    let counters = Arc::new(tokio::sync::Mutex::new((0usize, 0usize))); // (done, failed)
    let download_sem = Arc::new(Semaphore::new(concurrency.clamp(1, 32)));
    // Use only HALF the cores for the CPU-bound PDF/EPUB extraction so a large
    // run doesn't peg every core and lock up the whole machine (the UI/webview
    // and the OS need headroom). Capped at 4 — beyond that the disk + the
    // download side, not CPU, are the bottleneck anyway.
    let extract_sem = Arc::new(Semaphore::new((num_cpus::get() / 2).clamp(1, 4)));
    // Downloads use a dedicated client with NO overall timeout (only connect +
    // read-stall timeouts) so a large or slow PDF isn't aborted like the shared
    // API client would. See `make_download_client`.
    let download_client = Arc::new(make_download_client());

    let mut handles = Vec::with_capacity(candidates.len());
    for doc in candidates {
        let app = app.clone();
        let client = download_client.clone();
        let cancel = cancel.clone();
        let download_sem = download_sem.clone();
        let extract_sem = extract_sem.clone();
        let counters = counters.clone();
        let folder = folder.to_path_buf();
        let db_path = db_path.to_path_buf();
        let text_dir = text_dir.to_path_buf();
        let extract_flag = extract;
        let reuse_root = reuse_root.map(|p| p.to_path_buf());

        let handle = tokio::spawn(async move {
            let download_permit = match download_sem.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    // The download semaphore should never close mid-run; if it
                    // somehow does, count the doc as failed and emit so it doesn't
                    // silently vanish from the totals (done + failed < total).
                    tracing::error!("download semaphore closed, skipping {}: {e}", doc.url);
                    let (done, failed) = {
                        let mut c = counters.lock().await;
                        c.1 += 1;
                        (c.0, c.1)
                    };
                    let _ = app.emit(
                        EV_DOWNLOAD_FAILED,
                        DownloadFailedPayload {
                            doc,
                            error: format!("Couldn't start the download (internal error): {e}"),
                            done,
                            failed,
                            total,
                        },
                    );
                    return;
                }
            };
            // Announce the attempt BEFORE the cancel check so a download skipped
            // by a just-arrived Stop still shows up in the UI stream (instead of
            // the task vanishing silently). The terminal EV_CANCELLED clears any
            // leftover in-flight row.
            let _ = app.emit(
                EV_DOWNLOAD_STARTED,
                DownloadStartedPayload {
                    url: doc.url.clone(),
                    title: doc.title.clone(),
                    source: doc.source.clone(),
                },
            );
            if cancel.is_cancelled() {
                return;
            }

            // Cross-library reuse (opt-in): copy an identical file from a sibling
            // library instead of re-fetching. A hit is reported as `Cached` — no
            // network throughput, but it's still indexed + extracted below.
            let reused = match &reuse_root {
                Some(root) => super::downloader::reuse_from_siblings(&doc, &folder, root).await,
                None => None,
            };

            let app_for_progress = app.clone();
            let url_for_progress = doc.url.clone();
            let title_for_progress = doc.title.clone();
            let last_emit = std::sync::Mutex::new(std::time::Instant::now());

            let mut outcome = if let Some(reused_path) = reused {
                DownloadOutcome::Cached(reused_path)
            } else {
                download(&doc, &folder, &client, &cancel, |ev| {
                    // Throttle progress to ~5 updates/sec/file, but NEVER drop the
                    // terminal (`force`) event — that's the one that carries the
                    // true final byte count for fast / unknown-length downloads.
                    let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                    if !ev.force
                        && last.elapsed() < std::time::Duration::from_millis(200)
                        && (ev.total == 0 || ev.downloaded < ev.total)
                    {
                        return;
                    }
                    *last = std::time::Instant::now();
                    let _ = app_for_progress.emit(
                        EV_DOWNLOAD_PROGRESS,
                        DownloadProgressPayload {
                            url: url_for_progress.clone(),
                            title: title_for_progress.clone(),
                            downloaded: ev.downloaded,
                            total: ev.total,
                        },
                    );
                })
                .await
            };

            // Unpaywall fallback: if the download failed and the doc carries a
            // DOI, ask Unpaywall for an OA PDF and retry once. Recovers
            // DOI-bearing candidates whose primary link was paywalled, dead, or a
            // landing page with no resolvable PDF. Scoped to DOI-bearing docs, so
            // it never fires for the many web results that lack one.
            if matches!(outcome, DownloadOutcome::Failed(_)) && !cancel.is_cancelled() {
                if let Some(doi) =
                    super::dedup::extract_doi(doc.identifier.as_deref().unwrap_or(""))
                {
                    // Race the lookup against Stop. Unlike the rest of the task,
                    // resolve_oa_pdf_via_unpaywall takes no cancel token and is
                    // bounded only by its internal 8s timeout, so without this a
                    // Stop during this fallback could leave the "Stopping…" button
                    // hung for up to 8s per in-flight task.
                    let lookup = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => None,
                        r = super::downloader::resolve_oa_pdf_via_unpaywall(&client, &doi) => r,
                    };
                    if let Some(oa_url) = lookup {
                        if oa_url != doc.url {
                            let mut alt = doc.clone();
                            alt.url = oa_url;
                            let retry = download(&alt, &folder, &client, &cancel, |_ev| {}).await;
                            if !matches!(retry, DownloadOutcome::Failed(_)) {
                                outcome = retry;
                            }
                        }
                    }
                }
            }

            // Map the outcome to a saved/cached file (with a `cached` flag), or
            // handle the terminal failure/cancel cases and bail out of the task.
            let (path, cached) = match outcome {
                DownloadOutcome::Saved(path) => (path, false),
                DownloadOutcome::Cached(path) => (path, true),
                DownloadOutcome::Failed(err) => {
                    runlog::log(runlog::Event::DownloadFail {
                        source: &doc.source,
                        title: &doc.title,
                        url: &doc.url,
                        error: &err,
                    });
                    let (done, failed) = {
                        let mut c = counters.lock().await;
                        c.1 += 1;
                        (c.0, c.1)
                    };
                    let _ = app.emit(
                        EV_DOWNLOAD_FAILED,
                        DownloadFailedPayload {
                            doc,
                            error: err,
                            done,
                            failed,
                            total,
                        },
                    );
                    return;
                }
                DownloadOutcome::Cancelled => {
                    // Counted neither done nor failed; UI shows "cancelled" via cancel event.
                    return;
                }
            };

            {
                {
                    let bytes = tokio::fs::metadata(&path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    runlog::log(runlog::Event::DownloadOk {
                        source: &doc.source,
                        title: &doc.title,
                        url: &doc.url,
                        local_path: &path.to_string_lossy(),
                        bytes,
                    });
                    // Release the download permit before extraction so the
                    // next download can start. Extract uses its own
                    // CPU-count semaphore.
                    drop(download_permit);

                    let mut text_path: Option<String> = None;
                    let mut extract_error: Option<String> = None;
                    // Skip the CPU-bound text extraction if the user has hit Stop:
                    // the bytes are already on disk and the row is still persisted
                    // below, so short-circuiting only the optional parse keeps Stop
                    // responsive instead of grinding through queued large PDFs.
                    if extract_flag && !cancel.is_cancelled() {
                        // The extract semaphore should never close mid-run; if it
                        // somehow does, skip extraction but do NOT bare-return —
                        // that would orphan the already-downloaded file (no DB row,
                        // no count, no event, done+failed < total). Record it and
                        // fall through to persist + emit below.
                        let permit = match extract_sem.clone().acquire_owned().await {
                            Ok(p) => Some(p),
                            Err(e) => {
                                tracing::error!(
                                    "extract semaphore closed, skipping extraction for {}: {e}",
                                    doc.url
                                );
                                extract_error = Some(format!("text extraction unavailable: {e}"));
                                None
                            }
                        };
                        if let Some(_extract_permit) = permit {
                            let extract_path = path.clone();
                            let extract_res = match tokio::task::spawn_blocking(move || {
                                extract_text(&extract_path)
                            })
                            .await
                            {
                                Ok(res) => res,
                                Err(e) => Err(anyhow::anyhow!("extraction task panicked: {}", e)),
                            };
                            match extract_res {
                                Ok(text) => {
                                    if !text.trim().is_empty() {
                                        let stem = path
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("doc")
                                            .to_string();
                                        let tpath = text_dir.join(format!("{stem}.txt"));
                                        if tokio::fs::write(&tpath, text).await.is_ok() {
                                            if let Ok(rel) = tpath.strip_prefix(&folder) {
                                                text_path = Some(rel.to_string_lossy().to_string());
                                            }
                                        }
                                    } else {
                                        extract_error = Some("extracted text is empty".into());
                                    }
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    tracing::warn!(
                                        "extraction failed for {}: {}",
                                        path.display(),
                                        err_msg
                                    );
                                    extract_error = Some(err_msg);
                                }
                            }
                        }
                    }

                    let local_path = path
                        .strip_prefix(&folder)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());

                    // Persist to SQLite (open a fresh connection on the blocking thread
                    // because rusqlite::Connection is !Send and cannot cross .await points)
                    let db_path_for_task = db_path.clone();
                    let title_for_db = doc.title.clone();
                    let url_for_db = doc.url.clone();
                    let source_for_db = doc.source.clone();
                    let authors_for_db = doc.authors.join(", ");
                    let year_for_db = doc.year.clone();
                    let abstract_for_db = doc.abstract_.clone();
                    let lp_for_db = local_path.clone();
                    let tp_for_db = text_path.clone();
                    let ee_for_db = extract_error.clone();
                    // Persist the row, capturing any error so a SQLITE_BUSY / lock /
                    // constraint failure surfaces (tracing + run log + the UI's
                    // index_error) instead of silently dropping a downloaded doc.
                    let db_result = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
                        let mgr = DbManager::open_existing(&db_path_for_task)?;
                        mgr.insert_document(
                            run_id,
                            &title_for_db,
                            &url_for_db,
                            &source_for_db,
                            &authors_for_db,
                            year_for_db.as_deref(),
                            abstract_for_db.as_deref(),
                            &lp_for_db,
                            tp_for_db.as_deref(),
                            ee_for_db.as_deref(),
                            bytes,
                        )
                    })
                    .await;
                    let db_err = match db_result {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(join_err) => Some(format!("db task panicked: {join_err}")),
                    };
                    if let Some(err) = &db_err {
                        tracing::error!("failed to persist document row for {}: {}", doc.url, err);
                        runlog::log(runlog::Event::DbError {
                            title: &doc.title,
                            url: &doc.url,
                            error: err,
                        });
                    }

                    let (done, failed) = {
                        let mut c = counters.lock().await;
                        c.0 += 1;
                        (c.0, c.1)
                    };

                    let _ = app.emit(
                        EV_DOWNLOAD_DONE,
                        DownloadDonePayload {
                            doc,
                            local_path,
                            absolute_path: path.to_string_lossy().to_string(),
                            text_path,
                            bytes,
                            cached,
                            index_error: db_err,
                            extract_error,
                            done,
                            failed,
                            total,
                        },
                    );
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        if let Err(e) = h.await {
            if e.is_panic() {
                // A SINGLE download task panicked. This is NOT terminal for the
                // run — the other tasks are still downloading and the run finishes
                // normally via EV_COMPLETE. Emitting the global EV_ERROR here
                // flipped the whole run card to an "errored/ended" state mid-run
                // (then silently un-errored on EV_COMPLETE). Log it instead and
                // let the run complete; EV_ERROR is reserved for genuinely
                // terminal failures (run_pipeline returning Err). The lost task's
                // slot is reconciled visually by the finished-run progress state.
                tracing::error!("download task panicked (run continues): {e}");
            }
        }
    }

    let counts = counters.lock().await;
    *counts
}

/// Retry the download + extraction of a fixed list of documents into an existing
/// library folder, without re-running discovery or ranking. Emits the same
/// EV_DOWNLOAD_* / EV_COMPLETE events as a normal run so the UI tracks it like a
/// (scoped) run. Used by the "Retry failed" action.
pub async fn run_retry_pipeline(
    app: AppHandle,
    folder: PathBuf,
    query: String,
    docs: Vec<Document>,
    concurrency: usize,
    extract: bool,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let db_path = folder.join("library.db");
    let text_dir = folder.join("_text");
    if extract {
        tokio::fs::create_dir_all(&text_dir).await?;
    }
    // Reclaim any orphaned *.part temps before re-downloading into the folder.
    sweep_stale_parts(&folder).await;

    // Associate the retried rows with a fresh run in this library's db (the
    // folder always already has a library.db from the original run).
    let run_id = {
        let mgr = DbManager::new(&db_path).map_err(|e| anyhow::anyhow!("DB init failed: {}", e))?;
        mgr.insert_run(&query, &folder.to_string_lossy())
            .map_err(|e| anyhow::anyhow!("failed to insert run: {}", e))?
    };

    let total = docs.len();
    emit_stage(
        &app,
        "download",
        "started",
        None,
        Some(total as u64),
        Some(format!("{total} to retry")),
    );
    if extract {
        emit_stage(&app, "extract", "started", None, Some(total as u64), None);
    } else {
        emit_stage(
            &app,
            "extract",
            "skipped",
            None,
            None,
            Some("text extraction disabled".into()),
        );
    }

    // Retry is an explicit "fetch these again" action, so it does NOT use
    // cross-run reuse — that would just re-copy the stale sibling file we're
    // trying to replace. Pass None.
    let (done, failed) = download_and_extract(
        &app,
        docs,
        &folder,
        &text_dir,
        &db_path,
        run_id,
        concurrency,
        extract,
        None,
        &cancel,
    )
    .await;

    emit_stage(
        &app,
        "download",
        "done",
        Some(done as u64),
        Some(total as u64),
        Some(format!("{done} saved · {failed} failed")),
    );
    if extract {
        emit_stage(
            &app,
            "extract",
            "done",
            Some(done as u64),
            Some(total as u64),
            None,
        );
    }

    let folder_str = folder.to_string_lossy().to_string();
    runlog::log(runlog::Event::RunComplete {
        done,
        failed,
        total,
        folder: &folder_str,
    });
    let _ = app.emit(
        if cancel.is_cancelled() {
            EV_CANCELLED
        } else {
            EV_COMPLETE
        },
        CompletePayload {
            done,
            failed,
            total,
            folder: folder_str,
            manifest: db_path.to_string_lossy().to_string(),
        },
    );
    Ok(())
}

pub async fn run_pipeline(
    app: AppHandle,
    req: RunRequest,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let client = Arc::new(make_client());
    // Fast, regex-based sub-queries. Discovery runs on these IMMEDIATELY (wave
    // 1) so results stream within ~1s; the optional — and possibly slow, cold-
    // loading — LLM expansion runs concurrently and folds its extra sub-queries
    // in as wave 2. (Previously the LLM ran *before* discovery, so the whole run
    // sat at 0 found / 0 saved until the model finished loading + generating.)
    //
    // `expand_query_broad` (not the plain split) gives wave 1 a model-free
    // fan-out — drop-one keyword relaxations — so a fresh install with no LLM
    // still casts a wide net from the first search instead of running a single
    // narrow sub-query.
    let base = expand_query_broad(&req.query);

    runlog::log(runlog::Event::RunStart {
        query: &req.query,
        sub_queries: &base,
        sources: &req.sources,
    });
    // Emit keywords up front so the Sub-queries panel populates at t≈0 instead
    // of waiting on the LLM. Re-emitted with the LLM extras before wave 2.
    // Fire-and-forget (like every other emit here): a failed cosmetic event must
    // not abort the whole search — it previously used `?` and could.
    let _ = app.emit(
        EV_KEYWORDS,
        KeywordsPayload {
            query: req.query.clone(),
            sub_queries: base.clone(),
        },
    );

    let folder = PathBuf::from(&req.out_dir).join(safe_folder(&req.query));
    tokio::fs::create_dir_all(&folder).await?;
    // Reclaim any orphaned `*.part` temp files from a previous run killed
    // mid-download (SIGKILL/power-loss). The downloader cleans them on every
    // in-function error/cancel, but a hard kill leaves one behind and the
    // resume-skip check only inspects the FINAL path, so it would never be
    // reclaimed for a one-off query.
    sweep_stale_parts(&folder).await;
    let text_dir = folder.join("_text");
    if req.extract {
        tokio::fs::create_dir_all(&text_dir).await?;
    }

    let db_path = folder.join("library.db");
    let run_id = {
        let mgr = DbManager::new(&db_path).map_err(|e| anyhow::anyhow!("DB init failed: {}", e))?;
        mgr.insert_run(&req.query, &folder.to_string_lossy())
            .map_err(|e| anyhow::anyhow!("failed to insert run: {}", e))?
        // mgr (and its !Send Connection) is dropped here before any .await
    };

    // -------- Phase 1: parallel discovery -----------------------------------
    //
    // One spawned task per (sub_query, source). Each task drains its source
    // stream and pushes raw discoveries into a shared async Deduplicator. The
    // dedup happens incrementally so cross-source duplicates merge as they
    // arrive (one paper from arXiv + Semantic Scholar collapses to a single
    // candidate with both source attributions).
    //
    // EV_FOUND still fires per discovery to keep the existing live UI happy;
    // the new EV_CANDIDATE event fires once per merged candidate after
    // ranking, with rejection reasons attached.

    // The meta-search aggregator already fans out to the six web engines (and
    // falls back to SearXNG), so when it's enabled we drop those redundant
    // standalone web sources. Running both double-scrapes the same sites and
    // trips rate limits (403/429) without adding any coverage.
    const META_COVERED: &[&str] = &[
        "web",
        "brave",
        "bing",
        "mojeek",
        "marginalia",
        "startpage",
        "searxng",
    ];
    let effective_sources: Vec<String> = if req.sources.iter().any(|s| s == "meta_search") {
        req.sources
            .iter()
            .filter(|s| !META_COVERED.contains(&s.as_str()))
            .cloned()
            .collect()
    } else {
        req.sources.clone()
    };

    emit_stage(
        &app,
        "discovery",
        "started",
        None,
        None,
        Some(format!(
            "{} sources × {} sub-queries",
            effective_sources.len(),
            base.len()
        )),
    );

    let dedup: Arc<AsyncMutex<Deduplicator>> = Arc::new(AsyncMutex::new(Deduplicator::new()));
    // Aggregate keyword set for ranking later — accumulated across both waves.
    let mut all_keywords: Vec<String> = Vec::new();

    // One throttle per source, SHARED across both discovery waves so a wide
    // wave-2 fan-out can't burst a source that wave 1 (or a prior cap) already
    // had in flight. See `source_concurrency`.
    let source_sems: HashMap<String, Arc<Semaphore>> = effective_sources
        .iter()
        .map(|s| (s.clone(), Arc::new(Semaphore::new(source_concurrency(s)))))
        .collect();

    // Depth-scaled wave deadlines: a deeper run gives slow sources more time to
    // drain before the wave is force-stopped. See `wave_deadlines`.
    let (wave1_deadline, wave2_deadline) = wave_deadlines(req.max_total);

    // Wave 1 (base sub-queries) and the LLM expansion run CONCURRENTLY: the
    // expansion's cold model load / generation happens on the blocking pool
    // while discovery streams over the network, so the user sees `found` climb
    // immediately instead of staring at a frozen 0/0 card during the load.
    // `_stop_early` (wave 1 hit its deadline / drained naturally) no longer gates
    // wave 2 — see the wave-2 comment below. A user cancel is handled separately.
    let (_stop_early, llm_extras) = tokio::join!(
        discover_wave(
            &app,
            &client,
            &cancel,
            &dedup,
            &mut all_keywords,
            &base,
            &effective_sources,
            &req,
            wave1_deadline,
            &source_sems,
        ),
        compute_llm_extras(&app, &req, &cancel),
    );

    // Wave 2: fold in any LLM-derived sub-queries we don't already have, merging
    // into the same dedup. Gated ONLY on the user not having cancelled and there
    // still being room under max_total — deliberately NOT on whether wave 1 hit
    // its deadline. Previously a slow wave 1 that hit its deadline discarded the
    // entire LLM expansion, which is exactly backwards for a deep/thorough run
    // (the LLM fan-out is the main breadth multiplier). Skipping only when the
    // dedup is already full avoids spawning a wave that can add nothing.
    let room_left = dedup.lock().await.len() < req.max_total;
    if !cancel.is_cancelled() && room_left {
        let extras: Vec<String> = llm_extras
            .into_iter()
            .filter(|q| !base.iter().any(|b| b.eq_ignore_ascii_case(q)))
            .collect();
        if !extras.is_empty() {
            tracing::info!("discovery wave 2: {} LLM sub-queries", extras.len());
            let mut combined = base.clone();
            combined.extend(extras.iter().cloned());
            let _ = app.emit(
                EV_KEYWORDS,
                KeywordsPayload {
                    query: req.query.clone(),
                    sub_queries: combined,
                },
            );
            let _ = discover_wave(
                &app,
                &client,
                &cancel,
                &dedup,
                &mut all_keywords,
                &extras,
                &effective_sources,
                &req,
                // A wider fan-out needs a little more wall-clock to drain through
                // the per-source throttle; depth-scaled but still bounded so Stop
                // stays snappy.
                wave2_deadline,
                &source_sems,
            )
            .await;
        }
    }

    // ---------- Phase 1.5: dedup → rank → flag rejects ----------------------
    let merged = {
        let dlock = std::mem::take(&mut *dedup.lock().await);
        dlock.into_docs()
    };
    emit_stage(
        &app,
        "discovery",
        "done",
        Some(merged.len() as u64),
        None,
        Some(format!("{} unique candidates", merged.len())),
    );
    let _ = app.emit(
        EV_FOUND_TOTAL,
        FoundTotalPayload {
            count: merged.len(),
        },
    );

    // If the user cancelled during discovery, stop here: skip ranking, rerank,
    // LLM filter, candidate emission and downloads entirely so the UI flips to
    // "cancelled" immediately instead of grinding through the rest of the
    // pipeline. (Ranking and the EV_CANDIDATE loop below don't check the token,
    // so without this a cancelled run keeps streaming candidates and the found
    // partial set still gets download-skipped — looking like "nothing happens".)
    if cancel.is_cancelled() {
        let _ = app.emit(
            EV_CANCELLED,
            CompletePayload {
                done: 0,
                failed: 0,
                // Report the discovered count, not 0 — EV_FOUND_TOTAL already set
                // the frontend's `total` to merged.len(), and the cancelled handler
                // overwrites `total` from this payload. A 0 here would leave the UI
                // inconsistent (found=N, total=0) and break overallPct.
                total: merged.len(),
                folder: folder.to_string_lossy().to_string(),
                manifest: db_path.to_string_lossy().to_string(),
            },
        );
        return Ok(());
    }

    if merged.is_empty() {
        let payload = CompletePayload {
            done: 0,
            failed: 0,
            total: 0,
            folder: folder.to_string_lossy().to_string(),
            manifest: db_path.to_string_lossy().to_string(),
        };
        let _ = app.emit(
            if cancel.is_cancelled() {
                EV_CANCELLED
            } else {
                EV_COMPLETE
            },
            payload,
        );
        return Ok(());
    }

    // Rank everything we found, then flag rejects so the UI can show greyed
    // entries with explanations rather than silently dropping low-relevance
    // candidates.
    emit_stage(&app, "rank", "started", None, None, None);
    let mut ranked: Vec<RankedDoc> = flag_rejects(rank_candidates(&all_keywords, merged));
    let kept_after_rank = ranked.iter().filter(|r| r.reject_reason.is_none()).count();
    emit_stage(
        &app,
        "rank",
        "done",
        Some(kept_after_rank as u64),
        Some(ranked.len() as u64),
        Some(format!(
            "{} kept · {} rejected",
            kept_after_rank,
            ranked.len() - kept_after_rank
        )),
    );

    // Tier 2: optional semantic reranking via local embedding model.
    // Falls back silently to Tier 1 if the model isn't available — the
    // user gets no error, just lexical-only ranking.
    #[cfg(feature = "ai-embeddings")]
    if req.use_semantic_rerank && !cancel.is_cancelled() && crate::ai::embeddings::is_loaded() {
        // Depth-scaled rerank pool: re-embed up to a quarter of the discovery cap
        // (was a hardcoded 100), so a deep run semantically re-scores far more
        // than just the top 100. Clamped at 400 to keep the single embedding
        // batch (and its memory) bounded.
        let rerank_pool = (req.max_total / 4).clamp(100, 400);
        let to_rerank = ranked.len().min(rerank_pool) as u64;
        emit_stage(
            &app,
            "semantic_rerank",
            "started",
            None,
            Some(to_rerank),
            None,
        );
        let _ = app.emit(
            crate::events::EV_MODEL_STATUS,
            crate::events::ModelStatusPayload {
                model_id: "bge-small-en-v1.5".to_string(),
                status: "embedding".to_string(),
                detail: Some(format!("{} candidates", to_rerank)),
            },
        );
        let query_for_rerank = req.query.clone();
        let app_for_rerank = app.clone();
        // Keep a Tier-1 fallback: if the rerank task PANICS, `taken` is dropped
        // inside the dead thread and `ranked` would be left empty — meaning zero
        // candidates and zero downloads. Restore the lexical ranking instead.
        // The clone (≤ a few hundred RankedDocs) is a deliberate, cheap trade
        // against silently losing the entire run's results.
        let fallback = ranked.clone();
        let mut taken: Vec<RankedDoc> = std::mem::take(&mut ranked);
        let result = tokio::task::spawn_blocking(move || {
            let res = crate::ai::embeddings::rerank_blocking(
                &app_for_rerank,
                &query_for_rerank,
                &mut taken,
                rerank_pool,
            );
            (taken, res)
        })
        .await;
        match result {
            Ok((reranked, Ok(()))) => {
                ranked = reranked;
                emit_stage(
                    &app,
                    "semantic_rerank",
                    "done",
                    Some(to_rerank),
                    Some(to_rerank),
                    None,
                );
            }
            Ok((reranked, Err(e))) => {
                tracing::warn!("semantic rerank failed, falling back to Tier 1: {}", e);
                ranked = reranked;
                // We only entered this block because is_loaded() was true, so an
                // Err here means the worker became unavailable mid-rerank (e.g. the
                // documented macOS ort-abort latch). Emit a terminal embedding_failed
                // so the UI stops claiming "ready / + semantic rerank" for a model
                // that's actually disabled for the rest of the session, and offers a
                // retry — instead of every later search silently degrading to lexical.
                let _ = app.emit(
                    crate::events::EV_MODEL_STATUS,
                    crate::events::ModelStatusPayload {
                        model_id: "bge-small-en-v1.5".to_string(),
                        status: "embedding_failed".to_string(),
                        detail: Some(format!("rerank failed: {e}")),
                    },
                );
                emit_stage(
                    &app,
                    "semantic_rerank",
                    "skipped",
                    None,
                    None,
                    Some("model not loaded — using lexical only".into()),
                );
            }
            Err(e) => {
                tracing::warn!("rerank task panicked, falling back to Tier 1: {}", e);
                // `taken` died with the panicked thread; restore the lexical
                // ranking so the run still produces (correctly-ordered) downloads
                // instead of silently dropping every candidate.
                ranked = fallback;
                // A panic in the rerank thread means the model is unusable this
                // session — surface it (same reasoning as the Err arm above) so the
                // UI doesn't keep advertising semantic rerank as available.
                let _ = app.emit(
                    crate::events::EV_MODEL_STATUS,
                    crate::events::ModelStatusPayload {
                        model_id: "bge-small-en-v1.5".to_string(),
                        status: "embedding_failed".to_string(),
                        detail: Some(format!("rerank task panicked: {e}")),
                    },
                );
                emit_stage(
                    &app,
                    "semantic_rerank",
                    "skipped",
                    None,
                    None,
                    Some(format!("rerank task panicked — using lexical only: {}", e)),
                );
            }
        }
    }
    #[cfg(not(feature = "ai-embeddings"))]
    emit_stage(
        &app,
        "semantic_rerank",
        "skipped",
        None,
        None,
        Some("embeddings feature disabled at build time".into()),
    );
    #[cfg(feature = "ai-embeddings")]
    if !req.use_semantic_rerank {
        emit_stage(&app, "semantic_rerank", "skipped", None, None, None);
    } else if !cancel.is_cancelled() && !crate::ai::embeddings::is_loaded() {
        // First use of semantic rerank: load/download the model in the
        // background so this run doesn't stall on a cold ~60 MB download.
        // Semantic rerank engages automatically on the next search once ready.
        // Use the *implicit* warm so a search never retries a worker that has
        // already crashed this session (no crash loop).
        crate::ai::embeddings::warm_in_background_implicit(app.clone());
        emit_stage(
            &app,
            "semantic_rerank",
            "skipped",
            None,
            None,
            Some(
                "downloading semantic model in the background — used lexical ranking this run"
                    .into(),
            ),
        );
    }

    // Tier 3b: LLM borderline filter. Asks the LLM yes/no whether each
    // candidate in the 50–70th percentile band actually addresses the
    // query. Bounded scope = bounded latency.
    #[cfg(feature = "ai-llm")]
    if req.use_llm_filter && !cancel.is_cancelled() && ranked.len() >= 4 {
        emit_stage(&app, "llm_filter", "started", None, None, None);
        if let Some(llm) = ensure_llm_loaded(&app).await {
            // Determine the borderline band on the kept set only.
            let kept_indices: Vec<usize> = ranked
                .iter()
                .enumerate()
                .filter(|(_, r)| r.reject_reason.is_none())
                .map(|(i, _)| i)
                .collect();
            if kept_indices.len() >= 4 {
                let lower = kept_indices.len() * 50 / 100;
                let upper = kept_indices.len() * 70 / 100;
                let borderline: Vec<usize> = kept_indices[lower..upper.max(lower + 1)].to_vec();

                let _ = app.emit(
                    crate::events::EV_MODEL_STATUS,
                    crate::events::ModelStatusPayload {
                        model_id: "llm".to_string(),
                        status: "llm_filtering".to_string(),
                        detail: Some(format!("{} borderline candidates", borderline.len())),
                    },
                );

                // Build all prompts up-front, then pay the spawn_blocking +
                // mutex acquisition cost once. With ~10–20 borderline
                // candidates this collapses ~20 thread switches and lock
                // cycles into one. Cancellation is checked per-prompt
                // *inside* the blocking thread.
                let prompts: Vec<String> = borderline
                    .iter()
                    .map(|&idx| {
                        crate::ai::llm::filter_prompt(
                            &req.query,
                            &ranked[idx].doc.doc.title,
                            ranked[idx].doc.doc.abstract_.as_deref().unwrap_or(""),
                        )
                    })
                    .collect();
                let llm_for_task = llm.clone();
                // Same child-token pattern as the expansion/spellfix branches: a
                // parent Stop still propagates into the decode, and on the timeout
                // arm we cancel the child OURSELVES so the detached blocking task
                // bails at its next per-prompt check and RELEASES the singleton
                // model mutex. A bare `cancel.clone()` here (the prior code) was
                // never fired on timeout, so a slow decode kept the lock and
                // starved the NEXT run's LLM stages (expansion/spellfix/filter all
                // block on the same mutex) down to lexical-only / keep-all.
                let gen_cancel = cancel.child_token();
                let cancel_for_task = gen_cancel.clone();
                let handle = tokio::task::spawn_blocking(move || {
                    let guard = llm_for_task.blocking_lock();
                    let mut out = Vec::with_capacity(prompts.len());
                    for p in &prompts {
                        if cancel_for_task.is_cancelled() {
                            // Conservative: anything we didn't get to is kept.
                            out.extend(std::iter::repeat(true).take(prompts.len() - out.len()));
                            break;
                        }
                        out.push(guard.yes_no(p, &cancel_for_task).unwrap_or(true));
                    }
                    out
                });
                // Bound the filter: `blocking_lock()` has no timeout, so a wedged
                // decode holding the model mutex would otherwise stall the run
                // indefinitely. On cancel or a 30s timeout, conservatively KEEP all
                // borderline candidates. The user-cancel arm needs no explicit
                // child cancel (the parent token already propagated); the timeout
                // arm MUST cancel the child to free the lock from the runaway decode.
                let keeps = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => vec![true; borderline.len()],
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                        gen_cancel.cancel(); // free the model lock from the runaway decode
                        vec![true; borderline.len()]
                    }
                    r = handle => r.unwrap_or_else(|_| vec![true; borderline.len()]),
                };

                let mut rejected_by_llm = 0u64;
                for (i, idx) in borderline.iter().enumerate() {
                    if !keeps.get(i).copied().unwrap_or(true) {
                        ranked[*idx].reject_reason = Some("LLM judged off-topic".to_string());
                        rejected_by_llm += 1;
                    }
                }
                emit_stage(
                    &app,
                    "llm_filter",
                    "done",
                    Some(rejected_by_llm),
                    Some(borderline.len() as u64),
                    Some(format!(
                        "dropped {} of {} borderline",
                        rejected_by_llm,
                        borderline.len()
                    )),
                );
            } else {
                emit_stage(
                    &app,
                    "llm_filter",
                    "skipped",
                    None,
                    None,
                    Some("not enough kept candidates to filter".into()),
                );
            }
        } else {
            emit_stage(
                &app,
                "llm_filter",
                "skipped",
                None,
                None,
                Some("LLM model not downloaded".into()),
            );
        }
    } else {
        #[cfg(feature = "ai-llm")]
        emit_stage(&app, "llm_filter", "skipped", None, None, None);
    }
    #[cfg(not(feature = "ai-llm"))]
    emit_stage(
        &app,
        "llm_filter",
        "skipped",
        None,
        None,
        Some("LLM feature disabled at build time".into()),
    );

    // Tier 4: optional citation-graph enrichment. Boosts papers that are
    // referenced or cited by other top-scoring candidates. Off by default
    // because each enabled run hits Semantic Scholar's rate limits hard.
    if req.use_citation_graph && !cancel.is_cancelled() {
        emit_stage(&app, "citation_enrich", "started", None, None, None);
        ranked = enrich_with_citation_graph(client.clone(), ranked).await;
        // Re-flag rejects: scores changed, but the absolute TF-IDF cutoff
        // didn't, so this is mostly a no-op. Cheap to be defensive though.
        ranked = flag_rejects(ranked);
        emit_stage(&app, "citation_enrich", "done", None, None, None);
    } else {
        emit_stage(&app, "citation_enrich", "skipped", None, None, None);
    }

    // Emit one EV_CANDIDATE per ranked doc with full scoring breakdown +
    // reject_reason. This is what the multi-lane UI consumes for the
    // "All Found" tab (rejects greyed) and the "Downloading" tab (kept).
    let kept_count = ranked.iter().filter(|r| r.reject_reason.is_none()).count();
    let rejected_count = ranked.len() - kept_count;
    let mut kept_index = 0usize;
    for r in &ranked {
        let final_rank = if r.reject_reason.is_none() {
            kept_index += 1;
            Some(kept_index)
        } else {
            None
        };
        let _ = app.emit(
            EV_CANDIDATE,
            CandidatePayload {
                doc: r.doc.doc.clone(),
                sources: r.doc.sources(),
                tfidf: r.tfidf,
                rrf: r.rrf,
                authority: r.authority,
                score: r.score,
                status: if r.reject_reason.is_some() {
                    "rejected".to_string()
                } else {
                    "kept".to_string()
                },
                reject_reason: r.reject_reason.clone(),
                final_rank,
            },
        );
    }
    let _ = app.emit(
        EV_RANKING_DONE,
        RankingDonePayload {
            total_candidates: ranked.len(),
            kept: kept_count,
            rejected: rejected_count,
        },
    );

    // Convert kept ranked docs to the Document list Phase 2 expects, in
    // best-first order so the most relevant downloads start first.
    let candidates: Vec<Document> = ranked
        .into_iter()
        .filter(|r| r.reject_reason.is_none())
        .map(|r| r.doc.doc)
        .collect();

    // If the user hit Stop during the ranking / rerank / LLM-filter / citation
    // phases (which run to completion before checking the token), bail before
    // kicking off the download phase so the UI flips to "cancelled" promptly
    // instead of grinding through a no-op download/extract stage.
    if cancel.is_cancelled() {
        let _ = app.emit(
            EV_CANCELLED,
            CompletePayload {
                done: 0,
                failed: 0,
                // Kept candidates that would have been downloaded — keeps the
                // cancelled UI consistent with the found/total it was showing.
                total: candidates.len(),
                folder: folder.to_string_lossy().to_string(),
                manifest: db_path.to_string_lossy().to_string(),
            },
        );
        return Ok(());
    }

    emit_stage(
        &app,
        "download",
        "started",
        None,
        Some(candidates.len() as u64),
        Some(format!("{} kept candidates", candidates.len())),
    );
    if req.extract {
        emit_stage(
            &app,
            "extract",
            "started",
            None,
            Some(candidates.len() as u64),
            None,
        );
    } else {
        emit_stage(
            &app,
            "extract",
            "skipped",
            None,
            None,
            Some("text extraction disabled".into()),
        );
    }

    // -------- Phase 2: parallel downloads + parallel extraction --------
    // Delegated to the shared `download_and_extract` (also used by the
    // retry-failed path) so there is exactly one verified download path.
    let total = candidates.len();
    // Cross-run reuse scans sibling per-query folders under the library root
    // (the run folder's parent). Only enabled when the user opted in.
    let reuse_root = if req.cross_run_reuse {
        folder.parent()
    } else {
        None
    };
    let (done, failed) = download_and_extract(
        &app,
        candidates,
        &folder,
        &text_dir,
        &db_path,
        run_id,
        req.concurrency,
        req.extract,
        reuse_root,
        &cancel,
    )
    .await;

    emit_stage(
        &app,
        "download",
        "done",
        Some(done as u64),
        Some(total as u64),
        Some(format!("{} saved · {} failed", done, failed)),
    );
    if req.extract {
        emit_stage(
            &app,
            "extract",
            "done",
            Some(done as u64),
            Some(total as u64),
            None,
        );
    }

    let folder_str = folder.to_string_lossy().to_string();
    runlog::log(runlog::Event::RunComplete {
        done,
        failed,
        total,
        folder: &folder_str,
    });
    let payload = CompletePayload {
        done,
        failed,
        total,
        folder: folder_str,
        manifest: db_path.to_string_lossy().to_string(),
    };
    let _ = app.emit(
        if cancel.is_cancelled() {
            EV_CANCELLED
        } else {
            EV_COMPLETE
        },
        payload,
    );

    Ok(())
}

// =============================================================================
// LLM helpers (Tier 3) — only compiled when the ai-llm feature is enabled.
// =============================================================================

#[cfg(feature = "ai-llm")]
async fn ensure_llm_loaded(
    app: &AppHandle,
) -> Option<std::sync::Arc<tokio::sync::Mutex<crate::ai::llm::LlmModel>>> {
    if let Some(m) = crate::ai::llm::try_get() {
        return Some(m);
    }
    // Prefer the registry's default LLM (the 1.5B); but if its file isn't on
    // disk, fall back to ANY downloaded LLM — e.g. a RAM-limited user who, per
    // the 0.5B card's own advice, downloaded only the 0.5B. Without this the
    // backend silently skipped all LLM stages whenever the exact default file was
    // absent, while the UI's "any LLM ready" gating still enabled Thorough.
    // (Explicit UI-driven selection via RunRequest.llm_model_id is an E4 concern.)
    let on_disk = |e: &crate::ai::registry::ModelEntry| {
        crate::ai::storage::model_file(app, e)
            .map(|p| p.exists())
            .unwrap_or(false)
    };
    let entry = crate::ai::registry::default_for(crate::ai::ModelKind::Llm)
        .filter(|e| on_disk(e))
        .or_else(|| {
            crate::ai::registry::REGISTRY
                .iter()
                .find(|e| e.kind == crate::ai::ModelKind::Llm && on_disk(e))
        })?;
    let model_path = crate::ai::storage::model_file(app, entry).ok()?;
    if !model_path.exists() {
        return None;
    }

    // Verify the pinned SHA-256 before handing the file to the native loader. The
    // SHA was only checked at download time; a corrupt/truncated GGUF (disk rot,
    // an interrupted resume, tampering) can make llama.cpp abort natively — which
    // Rust can't catch. Runs once per session: the `try_get()` fast-path above
    // skips it after the first successful load.
    if !entry.sha256.is_empty() {
        match crate::ai::downloader::verify_sha256(&model_path, entry.sha256).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    "LLM {} failed SHA-256 verification — skipping load (corrupt/truncated); re-download from Settings",
                    entry.id
                );
                let _ = app.emit(
                    crate::events::EV_MODEL_STATUS,
                    crate::events::ModelStatusPayload {
                        model_id: entry.id.to_string(),
                        status: "failed".to_string(),
                        detail: Some(
                            "Model file failed its integrity check — re-download it.".to_string(),
                        ),
                    },
                );
                return None;
            }
            Err(e) => {
                tracing::warn!("LLM {} SHA-256 check errored: {e}", entry.id);
                return None;
            }
        }
    }

    let _ = app.emit(
        crate::events::EV_MODEL_STATUS,
        crate::events::ModelStatusPayload {
            model_id: entry.id.to_string(),
            status: "llm_warming".to_string(),
            detail: None,
        },
    );

    let path_clone = model_path.clone();
    let load = tokio::task::spawn_blocking(move || crate::ai::llm::load_blocking(&path_clone));
    // Bound the native mmap + metadata parse. This runs concurrently with discovery
    // wave 1 (and gates the LLM filter), so a wedged or pathologically slow load
    // must not hold the pipeline open. On timeout we skip the LLM this run —
    // graceful: expansion/filter simply don't contribute — without disabling it
    // for future runs. (The detached load finishes in the background, harmlessly.)
    let res = match tokio::time::timeout(std::time::Duration::from_secs(25), load).await {
        Ok(joined) => joined.ok()?,
        Err(_) => {
            tracing::warn!("LLM load timed out (>25s) — skipping LLM features this run");
            return None;
        }
    };
    match res {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("LLM load failed: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::wave_deadlines;

    #[test]
    fn wave_deadlines_scale_with_depth_and_clamp() {
        // Light (max_total 75): just above the 15/12 floor.
        let (w1, w2) = wave_deadlines(75);
        assert_eq!((w1.as_secs(), w2.as_secs()), (16, 13));
        // Balanced (500).
        let (w1, w2) = wave_deadlines(500);
        assert_eq!((w1.as_secs(), w2.as_secs()), (27, 22));
        // Deep (1500).
        let (w1, w2) = wave_deadlines(1500);
        assert_eq!((w1.as_secs(), w2.as_secs()), (52, 42));
        // Exhaustive (4000): clamps at the 60/45 ceiling.
        let (w1, w2) = wave_deadlines(4000);
        assert_eq!((w1.as_secs(), w2.as_secs()), (60, 45));
        // Degenerate tiny cap never drops below the floor.
        let (w1, w2) = wave_deadlines(0);
        assert_eq!((w1.as_secs(), w2.as_secs()), (15, 12));
    }
}
