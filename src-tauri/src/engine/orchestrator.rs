//! Core pipeline management for discovering, downloading, and bundling research documents.

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::citation_graph::enrich_with_citation_graph;
use super::db::DbManager;
use super::dedup::Deduplicator;
use super::downloader::{download, DownloadOutcome};
use super::extract::extract_text;
use super::query::{expand_query, parse_query, safe_folder};
use super::ranking::{flag_rejects, rank_candidates, RankedDoc};
use super::runlog;
use crate::events::*;
use crate::sources::{build_source, make_client, Document, SourceOptions, SOURCE_IDS};

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

pub async fn run_pipeline(
    app: AppHandle,
    req: RunRequest,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let client = Arc::new(make_client());
    let mut sub_queries = expand_query(&req.query);

    // Tier 3a: LLM query expansion (best-effort, gracefully no-op if model
    // isn't loaded). Runs before discovery so the new sub-queries hit every
    // source on the same parallel pass as the original.
    #[cfg(feature = "ai-llm")]
    {
        if req.use_llm_expansion && !cancel.is_cancelled() {
            emit_stage(&app, "llm_expand", "started", None, None, None);
            if let Some(llm) = ensure_llm_loaded(&app).await {
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
                let llm_for_task = llm.clone();
                let raw = tokio::task::spawn_blocking(move || {
                    // Async mutex inside a blocking thread — use blocking_lock.
                    let guard = llm_for_task.blocking_lock();
                    guard.generate(&prompt, 200)
                })
                .await;
                if let Ok(Ok(text)) = raw {
                    let extras = crate::ai::llm::parse_expansion(&text, &original);
                    let n = extras.len() as u64;
                    if n > 0 {
                        tracing::info!("LLM expanded query into {} extras", n);
                        sub_queries.extend(extras);
                    }
                    emit_stage(&app, "llm_expand", "done", Some(n), None,
                        if n > 0 { Some(format!("+{} sub-queries", n)) } else { None });
                } else {
                    emit_stage(&app, "llm_expand", "done", None, None, None);
                }
            } else {
                emit_stage(&app, "llm_expand", "skipped", None, None,
                    Some("LLM model not downloaded".into()));
            }
        } else {
            emit_stage(&app, "llm_expand", "skipped", None, None, None);
        }
    }
    #[cfg(not(feature = "ai-llm"))]
    emit_stage(&app, "llm_expand", "skipped", None, None,
        Some("LLM feature disabled at build time".into()));

    runlog::log(runlog::Event::RunStart {
        query: &req.query,
        sub_queries: &sub_queries,
        sources: &req.sources,
    });
    app.emit(
        EV_KEYWORDS,
        KeywordsPayload {
            query: req.query.clone(),
            sub_queries: sub_queries.clone(),
        },
    )?;

    let folder = PathBuf::from(&req.out_dir).join(safe_folder(&req.query));
    tokio::fs::create_dir_all(&folder).await?;
    let text_dir = folder.join("_text");
    if req.extract {
        tokio::fs::create_dir_all(&text_dir).await?;
    }

    let db_path = folder.join("library.db");
    let run_id = {
        let mgr = DbManager::new(&db_path)
            .map_err(|e| anyhow::anyhow!("DB init failed: {}", e))?;
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

    emit_stage(&app, "discovery", "started", None, None,
        Some(format!("{} sources × {} sub-queries", req.sources.len(), sub_queries.len())));

    let dedup: Arc<AsyncMutex<Deduplicator>> = Arc::new(AsyncMutex::new(Deduplicator::new()));
    // Aggregate per-(sub_query) keyword set for ranking later.
    let mut all_keywords: Vec<String> = Vec::new();
    let mut tasks: JoinSet<()> = JoinSet::new();

    for sub in &sub_queries {
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

        for sname in &req.sources {
            if !SOURCE_IDS.contains(&sname.as_str()) {
                let _ = app.emit(
                    EV_SOURCE_ERROR,
                    SourceErrorPayload {
                        source: sname.clone(),
                        error: "unknown source".into(),
                    },
                );
                continue;
            }
            let opts = req.source_options.get(sname).cloned().unwrap_or_default();
            let Some(src) = build_source(sname, opts, client.clone()) else {
                continue;
            };

            let app_t = app.clone();
            let cancel_t = cancel.clone();
            let dedup_t = dedup.clone();
            let sub_t = sub.clone();
            let sname_t = sname.clone();
            let keywords_t = keywords.clone();
            let per_source = req.per_source;
            let max_total = req.max_total;

            tasks.spawn(async move {
                let _ = app_t.emit(
                    EV_SOURCE_START,
                    SourceStartPayload {
                        source: sname_t.clone(),
                        sub_query: sub_t.clone(),
                    },
                );

                let mut count = 0usize;
                let mut rank_in_source = 0usize;
                let mut stream = src.search(keywords_t.clone(), per_source).await;

                while let Some(item) = stream.next().await {
                    if cancel_t.is_cancelled() {
                        break;
                    }
                    match item {
                        Ok(doc) => {
                            rank_in_source += 1;
                            // Hard cap so a slow source can't push us past max_total.
                            // Cheaply checked under the lock below.
                            let total_now = {
                                let mut d = dedup_t.lock().await;
                                if d.len() >= max_total {
                                    break;
                                }
                                let title_for_evt = doc.title.clone();
                                let url_for_evt = doc.url.clone();
                                let source_for_evt = doc.source.clone();
                                d.add(doc, &sname_t, rank_in_source);
                                count += 1;
                                let total = d.len();
                                drop(d);
                                let _ = app_t.emit(
                                    EV_FOUND,
                                    FoundPayload {
                                        title: title_for_evt,
                                        source: source_for_evt,
                                        url: url_for_evt,
                                        total,
                                    },
                                );
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
                            let _ = app_t.emit(
                                EV_SOURCE_ERROR,
                                SourceErrorPayload {
                                    source: sname_t.clone(),
                                    error: err_str,
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

    // Drain all spawned discovery tasks. Errors here are panics from the
    // task body — they shouldn't happen since each task only emits events
    // and pushes into the mutex.
    while let Some(res) = tasks.join_next().await {
        if let Err(e) = res {
            tracing::warn!("discovery task join error: {e}");
        }
    }

    // ---------- Phase 1.5: dedup → rank → flag rejects ----------------------
    let merged = {
        let dlock = std::mem::take(&mut *dedup.lock().await);
        dlock.into_docs()
    };
    emit_stage(&app, "discovery", "done", Some(merged.len() as u64), None,
        Some(format!("{} unique candidates", merged.len())));
    let _ = app.emit(
        EV_FOUND_TOTAL,
        FoundTotalPayload {
            count: merged.len(),
        },
    );

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
    // candidates the way the old `relevance_score == 0` filter did.
    emit_stage(&app, "rank", "started", None, None, None);
    let mut ranked: Vec<RankedDoc> = flag_rejects(rank_candidates(&all_keywords, merged));
    let kept_after_rank = ranked.iter().filter(|r| r.reject_reason.is_none()).count();
    emit_stage(&app, "rank", "done", Some(kept_after_rank as u64), Some(ranked.len() as u64),
        Some(format!("{} kept · {} rejected", kept_after_rank, ranked.len() - kept_after_rank)));

    // Tier 2: optional semantic reranking via local embedding model.
    // Falls back silently to Tier 1 if the model isn't available — the
    // user gets no error, just lexical-only ranking.
    #[cfg(feature = "ai-embeddings")]
    if req.use_semantic_rerank && !cancel.is_cancelled() {
        let to_rerank = ranked.len().min(100) as u64;
        emit_stage(&app, "semantic_rerank", "started", None, Some(to_rerank), None);
        let _ = app.emit(
            crate::events::EV_MODEL_STATUS,
            crate::events::ModelStatusPayload {
                model_id: "bge-small-en-v1.5".to_string(),
                status: "embedding".to_string(),
                detail: Some(format!("{} candidates", to_rerank)),
            },
        );
        let query_for_rerank = req.query.clone();
        let mut taken: Vec<RankedDoc> = std::mem::take(&mut ranked);
        let result = tokio::task::spawn_blocking(move || {
            let res = crate::ai::embeddings::rerank_blocking(&query_for_rerank, &mut taken, 100);
            (taken, res)
        })
        .await;
        match result {
            Ok((reranked, Ok(()))) => {
                ranked = reranked;
                emit_stage(&app, "semantic_rerank", "done", Some(to_rerank), Some(to_rerank), None);
            }
            Ok((reranked, Err(e))) => {
                tracing::warn!("semantic rerank failed, falling back to Tier 1: {}", e);
                ranked = reranked;
                emit_stage(&app, "semantic_rerank", "skipped", None, None,
                    Some("model not loaded — using lexical only".into()));
            }
            Err(e) => {
                tracing::warn!("rerank task panicked: {}", e);
                emit_stage(&app, "semantic_rerank", "skipped", None, None,
                    Some(format!("rerank task panicked: {}", e)));
                // ranked is empty here because of std::mem::take — that's fine
                // because the task only panics after taking ownership.
            }
        }
    }
    #[cfg(not(feature = "ai-embeddings"))]
    emit_stage(&app, "semantic_rerank", "skipped", None, None,
        Some("embeddings feature disabled at build time".into()));
    #[cfg(feature = "ai-embeddings")]
    if !req.use_semantic_rerank {
        emit_stage(&app, "semantic_rerank", "skipped", None, None, None);
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
                let cancel_for_task = cancel.clone();
                let keeps = tokio::task::spawn_blocking(move || {
                    let guard = llm_for_task.blocking_lock();
                    let mut out = Vec::with_capacity(prompts.len());
                    for p in &prompts {
                        if cancel_for_task.is_cancelled() {
                            // Conservative: anything we didn't get to is kept.
                            out.extend(std::iter::repeat(true).take(prompts.len() - out.len()));
                            break;
                        }
                        out.push(guard.yes_no(p).unwrap_or(true));
                    }
                    out
                })
                .await
                .unwrap_or_else(|_| vec![true; borderline.len()]);

                let mut rejected_by_llm = 0u64;
                for (i, idx) in borderline.iter().enumerate() {
                    if !keeps.get(i).copied().unwrap_or(true) {
                        ranked[*idx].reject_reason = Some("LLM judged off-topic".to_string());
                        rejected_by_llm += 1;
                    }
                }
                emit_stage(&app, "llm_filter", "done", Some(rejected_by_llm),
                    Some(borderline.len() as u64),
                    Some(format!("dropped {} of {} borderline", rejected_by_llm, borderline.len())));
            } else {
                emit_stage(&app, "llm_filter", "skipped", None, None,
                    Some("not enough kept candidates to filter".into()));
            }
        } else {
            emit_stage(&app, "llm_filter", "skipped", None, None,
                Some("LLM model not downloaded".into()));
        }
    } else {
        #[cfg(feature = "ai-llm")]
        emit_stage(&app, "llm_filter", "skipped", None, None, None);
    }
    #[cfg(not(feature = "ai-llm"))]
    emit_stage(&app, "llm_filter", "skipped", None, None,
        Some("LLM feature disabled at build time".into()));

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

    emit_stage(&app, "download", "started", None, Some(candidates.len() as u64),
        Some(format!("{} kept candidates", candidates.len())));
    if req.extract {
        emit_stage(&app, "extract", "started", None, Some(candidates.len() as u64), None);
    } else {
        emit_stage(&app, "extract", "skipped", None, None,
            Some("text extraction disabled".into()));
    }

    // -------- Phase 2: parallel downloads + parallel extraction --------
    //
    // Two separate semaphores so download (network-bound) and extract
    // (CPU-bound) don't serialize each other. Previously a single semaphore
    // gated the whole task lifetime — once a download finished, the same
    // permit was held through extraction, blocking the next download from
    // starting. Now the download permit drops the moment bytes are on disk,
    // and a smaller extract permit (CPU count) gates the heavy text parsing.
    let total = candidates.len();
    let counters = Arc::new(tokio::sync::Mutex::new((0usize, 0usize))); // (done, failed)
    let download_sem = Arc::new(Semaphore::new(req.concurrency.clamp(1, 32)));
    let extract_sem = Arc::new(Semaphore::new(num_cpus::get().clamp(1, 8)));

    let mut handles = Vec::with_capacity(candidates.len());
    for doc in candidates {
        let app = app.clone();
        let client = client.clone();
        let cancel = cancel.clone();
        let download_sem = download_sem.clone();
        let extract_sem = extract_sem.clone();
        let counters = counters.clone();
        let folder = folder.clone();
        let db_path = db_path.clone();
        let text_dir = text_dir.clone();
        let extract_flag = req.extract;

        let handle = tokio::spawn(async move {
            let download_permit = match download_sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            if cancel.is_cancelled() {
                return;
            }

            let _ = app.emit(
                EV_DOWNLOAD_STARTED,
                DownloadStartedPayload {
                    url: doc.url.clone(),
                    title: doc.title.clone(),
                    source: doc.source.clone(),
                },
            );

            let app_for_progress = app.clone();
            let url_for_progress = doc.url.clone();
            let title_for_progress = doc.title.clone();
            let last_emit = std::sync::Mutex::new(std::time::Instant::now());

            let outcome = download(&doc, &folder, &client, &cancel, |ev| {
                // Throttle progress to ~5 updates/sec/file
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() < std::time::Duration::from_millis(200)
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
            .await;

            match outcome {
                DownloadOutcome::Saved(path) => {
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
                    if extract_flag {
                        let extract_path = path.clone();
                        let _extract_permit = match extract_sem.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => return,
                        };
                        let extract_res =
                            match tokio::task::spawn_blocking(move || extract_text(&extract_path))
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
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(mgr) = DbManager::new(&db_path_for_task) {
                            let _ = mgr.insert_document(
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
                            );
                        }
                    })
                    .await;

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
                            done,
                            failed,
                            total,
                        },
                    );
                }
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
                }
                DownloadOutcome::Cancelled => {
                    // Counted neither done nor failed; UI shows "cancelled" via cancel event.
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        if let Err(e) = h.await {
            if e.is_panic() {
                let msg = format!("download task panicked: {e}");
                tracing::error!("{msg}");
                let _ = app.emit(EV_ERROR, ErrorPayload { message: msg });
            }
        }
    }

    let (done, failed) = {
        let c = counters.lock().await;
        *c
    };
    emit_stage(&app, "download", "done", Some(done as u64), Some(total as u64),
        Some(format!("{} saved · {} failed", done, failed)));
    if req.extract {
        emit_stage(&app, "extract", "done", Some(done as u64), Some(total as u64), None);
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
    // Pick the registry's default LLM. UI-driven model selection would
    // override this via RunRequest.llm_model_id but that's an E4 concern.
    let entry = crate::ai::registry::default_for(crate::ai::ModelKind::Llm)?;
    let model_path = crate::ai::storage::model_file(app, entry).ok()?;
    if !model_path.exists() {
        return None;
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
    let res = tokio::task::spawn_blocking(move || crate::ai::llm::load_blocking(&path_clone))
        .await
        .ok()?;
    match res {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("LLM load failed: {}", e);
            None
        }
    }
}
