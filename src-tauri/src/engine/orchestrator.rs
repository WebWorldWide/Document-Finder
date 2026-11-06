//! The search → rank → download → read pipeline.
//!
//! Everything runs concurrently and adapts itself — there are no user knobs:
//!
//! 1. **Discovery wave 1** fans the query's sub-queries out to every enabled
//!    source (per-source concurrency + a fair per-source share of the
//!    candidate pool so one fast source can't crowd out the rest).
//! 2. A **dispatcher** re-ranks the growing candidate pool (BM25F, see
//!    `ranking`) every ~1 s and feeds a best-first queue. Downloads start
//!    within seconds instead of waiting for discovery to finish, and a strong
//!    late result jumps ahead of weaker queued ones.
//! 3. After wave 1, **pseudo-relevance feedback** mines the best results for
//!    the field's own vocabulary and runs **wave 2** — only when the first
//!    wave didn't already find plenty. A bounded **citation-graph** pass runs
//!    alongside and re-orders whatever is still queued.
//! 4. **Download workers** pull from the queue under a per-host adaptive
//!    limiter, try every known URL + Unpaywall, reuse files already present
//!    anywhere in the library, then extract Markdown + chunks.
//! 5. A single **writer thread** batches rows into SQLite.

use futures::StreamExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::{oneshot, Notify, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::citation_graph::citation_boosts;
use super::db::{ChunkRecord, DbManager, DocRecord, LibraryIndex};
use super::dedup::{AddOutcome, Deduplicator};
use super::downloader::{download, reuse_file, DownloadOutcome};
use super::extract::{extract, front_matter, Extracted};
use super::hostlimit::HostLimiter;
use super::query::{expand_query_broad, parse_query, safe_folder};
use super::ranking::{
    expansion_terms, feedback_subqueries, rank_candidates, QueryModel, RankedDoc,
};
use super::runlog;
use crate::events::*;
use crate::sources::{
    build_source, make_client, make_download_client, Document, SourceOptions, SOURCE_IDS,
};

/// A search request. Only *what* to search is configurable; *how much* and
/// *how fast* are decided by the pipeline.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunRequest {
    pub query: String,
    pub sources: Vec<String>,
    pub out_dir: String,
    #[serde(default)]
    pub source_options: HashMap<String, SourceOptions>,
}

// ---- Internal budgets (deliberately not user-facing) ----------------------

/// Unique candidates we stop discovering at.
const MAX_CANDIDATES: usize = 1500;
/// Most documents one search will download.
pub const MAX_DOWNLOADS: usize = 300;
/// If wave 1 already yields this many kept candidates, skip the feedback wave.
const ENOUGH_KEPT: usize = 450;
/// Concurrent download workers (per-host limits keep this polite).
const DOWNLOAD_WORKERS: usize = 24;
const WAVE1_DEADLINE: Duration = Duration::from_secs(30);
const WAVE2_DEADLINE: Duration = Duration::from_secs(20);
/// Downloads begin once this many candidates exist (or after WARMUP_TIME).
const WARMUP_MIN: usize = 30;
const WARMUP_TIME: Duration = Duration::from_secs(3);
const RERANK_EVERY: Duration = Duration::from_millis(1000);
/// While discovery is still running, only the top share of kept candidates
/// is dispatched (early IDF statistics are noisy).
const EARLY_DISPATCH_SHARE: f32 = 0.6;
const CITATION_BUDGET: Duration = Duration::from_secs(12);

/// Results requested per (sub-query, source) stream.
fn per_source_limit(name: &str) -> usize {
    match name {
        "meta_search" | "searxng" => 30,
        "gutenberg" => 20,
        "internet_archive" => 40,
        _ => 60,
    }
}

/// Concurrent streams per source across all sub-queries.
fn source_concurrency(name: &str) -> usize {
    match name {
        // arXiv's API terms: one request every 3 s — never parallelize.
        "arxiv" => 1,
        // Fans out to six engines internally / rate-limits hard.
        "meta_search" | "searxng" | "web" => 2,
        "semantic_scholar" | "zenodo" => 2,
        "internet_archive" => 2,
        _ => 4,
    }
}

const WEB_SCRAPERS: &[&str] = &[
    "web",
    "brave",
    "bing",
    "mojeek",
    "marginalia",
    "startpage",
    "meta_search",
    "searxng",
];

fn emit_stage(
    sink: &dyn Sink,
    stage: &str,
    state: &str,
    count: Option<u64>,
    total: Option<u64>,
    message: Option<String>,
) {
    emit(
        sink,
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

// ---------------------------------------------------------------------------
// Shared run state
// ---------------------------------------------------------------------------

struct Queued {
    idx: usize,
    score: f32,
    doc: Document,
    sources: Vec<String>,
}

struct RunState {
    dedup: Mutex<Deduplicator>,
    /// Candidate indices already handed to a worker.
    started: Mutex<HashSet<usize>>,
    /// Best-first queue, rebuilt by the dispatcher on every re-rank.
    queue: Mutex<Vec<Queued>>,
    queue_notify: Notify,
    /// No more candidates will be added (discovery finished / cancelled).
    discovery_done: AtomicBool,
    /// The dispatcher has made its final pass; workers exit when idle.
    dispatch_done: AtomicBool,
    boosts: Mutex<HashMap<String, f32>>,
    query: Mutex<QueryModel>,
    /// New unique docs contributed per source (fair-share accounting).
    per_source_new: Mutex<HashMap<String, usize>>,
    source_quota: usize,
    planned: AtomicUsize,
    done: AtomicUsize,
    failed: AtomicUsize,
    /// sha256 → title of files saved this run (duplicate-content guard).
    hashes: Mutex<HashMap<String, String>>,
}

impl RunState {
    fn new(query: QueryModel, n_sources: usize) -> Self {
        Self {
            dedup: Mutex::new(Deduplicator::new()),
            started: Mutex::new(HashSet::new()),
            queue: Mutex::new(Vec::new()),
            queue_notify: Notify::new(),
            discovery_done: AtomicBool::new(false),
            dispatch_done: AtomicBool::new(false),
            boosts: Mutex::new(HashMap::new()),
            query: Mutex::new(query),
            per_source_new: Mutex::new(HashMap::new()),
            // Each source may fill up to 2.5× an equal share of the pool, so a
            // single fast source can't crowd out the others.
            source_quota: ((MAX_CANDIDATES as f32 / n_sources.max(1) as f32) * 2.5) as usize,
            planned: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            hashes: Mutex::new(HashMap::new()),
        }
    }

    fn rank_now(&self) -> Vec<(usize, RankedDoc)> {
        let snapshot = self.dedup.lock().snapshot();
        let q = self.query.lock().clone();
        let boosts = self.boosts.lock().clone();
        // Remember each doc's dedup index via its primary URL (stable: the
        // dedup only appends, and promotes URLs only within an entry).
        let idx_of: HashMap<String, usize> = snapshot
            .iter()
            .enumerate()
            .map(|(i, m)| (m.doc.url.clone(), i))
            .collect();
        rank_candidates(&q, snapshot, &boosts)
            .into_iter()
            .filter_map(|r| idx_of.get(&r.doc.doc.url).map(|&i| (i, r)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn discover_wave(
    sink: &Arc<dyn Sink>,
    app: &Option<AppHandle>,
    client: &Arc<reqwest::Client>,
    cancel: &CancellationToken,
    state: &Arc<RunState>,
    sub_queries: &[String],
    sources: &[String],
    req: &RunRequest,
    deadline: Duration,
    source_sems: &HashMap<String, Arc<Semaphore>>,
) {
    let mut tasks: JoinSet<()> = JoinSet::new();
    for sub in sub_queries {
        let mut keywords = parse_query(sub);
        if keywords.is_empty() {
            keywords = sub.split_whitespace().map(String::from).collect();
        }
        if keywords.is_empty() {
            continue;
        }
        emit(
            sink.as_ref(),
            EV_SUBQUERY_START,
            SubQueryStartPayload {
                sub_query: sub.clone(),
                keywords: keywords.clone(),
            },
        );
        for sname in sources {
            if !SOURCE_IDS.contains(&sname.as_str()) {
                continue;
            }
            let opts = req.source_options.get(sname).cloned().unwrap_or_default();
            let Some(src) = build_source(sname, opts, Arc::clone(client), app.clone()) else {
                continue;
            };
            let sink = Arc::clone(sink);
            let cancel = cancel.clone();
            let state = Arc::clone(state);
            let sub = sub.clone();
            let sname = sname.clone();
            let keywords = keywords.clone();
            let sem = source_sems.get(&sname).cloned();
            tasks.spawn(async move {
                let _permit = match sem {
                    Some(s) => s.acquire_owned().await.ok(),
                    None => None,
                };
                if cancel.is_cancelled() {
                    return;
                }
                let quota_hit = |state: &RunState, sname: &str| {
                    state.per_source_new.lock().get(sname).copied().unwrap_or(0)
                        >= state.source_quota
                };
                if quota_hit(&state, &sname) {
                    return;
                }
                emit(
                    sink.as_ref(),
                    EV_SOURCE_START,
                    SourceStartPayload {
                        source: sname.clone(),
                        sub_query: sub.clone(),
                    },
                );
                let mut count = 0usize;
                let mut rank = 0usize;
                let mut seen_kinds: HashSet<&'static str> = HashSet::new();
                let mut stream = src.search(keywords, per_source_limit(&sname)).await;
                loop {
                    let item = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        i = stream.next() => i,
                    };
                    let Some(item) = item else { break };
                    match item {
                        Ok(doc) => {
                            rank += 1;
                            let (outcome, total) = {
                                let mut d = state.dedup.lock();
                                if d.len() >= MAX_CANDIDATES {
                                    break;
                                }
                                let title = doc.title.clone();
                                let url = doc.url.clone();
                                let source = doc.source.clone();
                                let o = d.add(doc, &sname, rank);
                                (
                                    matches!(o, AddOutcome::New(_)).then_some((title, url, source)),
                                    d.len(),
                                )
                            };
                            if let Some((title, url, source)) = outcome {
                                count += 1;
                                *state
                                    .per_source_new
                                    .lock()
                                    .entry(sname.clone())
                                    .or_insert(0) += 1;
                                emit(
                                    sink.as_ref(),
                                    EV_FOUND,
                                    FoundPayload {
                                        title,
                                        source,
                                        url,
                                        total,
                                    },
                                );
                                if quota_hit(&state, &sname) {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            let err = e.to_string();
                            runlog::log(runlog::Event::SourceError {
                                source: &sname,
                                error: &err,
                                sub_query: Some(&sub),
                            });
                            let kind = classify_source_error(&err);
                            // Markup drift in an HTML scraper isn't actionable.
                            if kind == "parse_error" && WEB_SCRAPERS.contains(&sname.as_str()) {
                                continue;
                            }
                            if !seen_kinds.insert(kind) {
                                continue;
                            }
                            emit(
                                sink.as_ref(),
                                EV_SOURCE_ERROR,
                                SourceErrorPayload {
                                    source: sname.clone(),
                                    error: err,
                                    kind: kind.into(),
                                },
                            );
                        }
                    }
                }
                emit(
                    sink.as_ref(),
                    EV_SOURCE_DONE,
                    SourceDonePayload {
                        source: sname.clone(),
                        count,
                    },
                );
            });
        }
    }

    let stopped_early = tokio::select! {
        biased;
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(deadline) => true,
        _ = async {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if state.dedup.lock().len() >= MAX_CANDIDATES {
                    break;
                }
            }
        } => true,
        _ = async {
            while let Some(r) = tasks.join_next().await {
                if let Err(e) = r {
                    if e.is_panic() {
                        tracing::warn!("discovery task panicked: {e}");
                    }
                }
            }
        } => false,
    };
    if stopped_early {
        tasks.shutdown().await;
    }
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Re-rank the pool and rebuild the best-first queue. Returns the ranking.
fn dispatch_pass(state: &RunState, sink: &dyn Sink, final_pass: bool) -> Vec<(usize, RankedDoc)> {
    let ranked = state.rank_now();
    let kept: Vec<&(usize, RankedDoc)> = ranked
        .iter()
        .filter(|(_, r)| r.reject_reason.is_none())
        .collect();
    let eligible = if final_pass {
        kept.len()
    } else {
        ((kept.len() as f32) * EARLY_DISPATCH_SHARE).ceil() as usize
    };
    let started = state.started.lock().clone();
    let room = MAX_DOWNLOADS.saturating_sub(started.len());
    let queue: Vec<Queued> = kept
        .iter()
        .take(eligible)
        .filter(|(i, _)| !started.contains(i))
        .take(room)
        .map(|(i, r)| Queued {
            idx: *i,
            score: r.score,
            doc: r.doc.doc.clone(),
            sources: r.doc.sources(),
        })
        .collect();
    let planned = (started.len() + queue.len()).min(MAX_DOWNLOADS);
    let planned = if final_pass {
        planned
    } else {
        planned.max(state.planned.load(Ordering::Relaxed))
    };
    if state.planned.swap(planned, Ordering::Relaxed) != planned {
        emit(sink, EV_PLAN, PlanPayload { planned });
    }
    *state.queue.lock() = queue;
    state.queue_notify.notify_waiters();
    ranked
}

fn emit_candidates(sink: &dyn Sink, ranked: &[(usize, RankedDoc)]) {
    let mut kept_index = 0usize;
    for (_, r) in ranked {
        let final_rank = if r.reject_reason.is_none() {
            kept_index += 1;
            Some(kept_index)
        } else {
            None
        };
        emit(
            sink,
            EV_CANDIDATE,
            CandidatePayload {
                doc: r.doc.doc.clone(),
                sources: r.doc.sources(),
                relevance: r.relevance,
                rrf: r.rrf,
                authority: r.authority,
                score: r.score,
                status: if r.reject_reason.is_some() {
                    "rejected".into()
                } else {
                    "kept".into()
                },
                reject_reason: r.reject_reason.clone(),
                final_rank,
            },
        );
    }
    let kept = ranked
        .iter()
        .filter(|(_, r)| r.reject_reason.is_none())
        .count();
    emit(
        sink,
        EV_RANKING_DONE,
        RankingDonePayload {
            total_candidates: ranked.len(),
            kept,
            rejected: ranked.len() - kept,
        },
    );
}

// ---------------------------------------------------------------------------
// Writer thread
// ---------------------------------------------------------------------------

type WriteJob = (
    DocRecord,
    Vec<ChunkRecord>,
    oneshot::Sender<Result<i64, String>>,
);

/// One SQLite connection on a dedicated thread; batches whatever is queued
/// into a single transaction.
fn spawn_writer(db_path: PathBuf, run_id: i64) -> tokio::sync::mpsc::UnboundedSender<WriteJob> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WriteJob>();
    std::thread::spawn(move || {
        let mut mgr = match DbManager::open_existing(&db_path) {
            Ok(m) => m,
            Err(e) => {
                while let Some((_, _, reply)) = rx.blocking_recv() {
                    let _ = reply.send(Err(e.to_string()));
                }
                return;
            }
        };
        while let Some(first) = rx.blocking_recv() {
            let mut jobs = vec![first];
            while jobs.len() < 64 {
                match rx.try_recv() {
                    Ok(j) => jobs.push(j),
                    Err(_) => break,
                }
            }
            let items: Vec<(DocRecord, Vec<ChunkRecord>)> = jobs
                .iter()
                .map(|(d, c, _)| (d.clone(), c.clone()))
                .collect();
            match mgr.upsert_batch(run_id, &items) {
                Ok(ids) => {
                    for ((_, _, reply), id) in jobs.into_iter().zip(ids) {
                        let _ = reply.send(Ok(id));
                    }
                }
                Err(_) => {
                    // Fall back to one-by-one so one bad row can't sink the batch.
                    for (d, c, reply) in jobs {
                        let r = mgr
                            .upsert_document(run_id, &d, &c)
                            .map_err(|e| e.to_string());
                        let _ = reply.send(r);
                    }
                }
            }
        }
    });
    tx
}

// ---------------------------------------------------------------------------
// Download workers
// ---------------------------------------------------------------------------

struct WorkerCtx {
    sink: Arc<dyn Sink>,
    state: Arc<RunState>,
    cancel: CancellationToken,
    client: Arc<reqwest::Client>,
    limiter: Arc<HostLimiter>,
    folder: PathBuf,
    text_dir: PathBuf,
    writer: tokio::sync::mpsc::UnboundedSender<WriteJob>,
    index: Arc<Mutex<Option<LibraryIndex>>>,
    extract_sem: Arc<Semaphore>,
}

async fn next_job(ctx: &WorkerCtx) -> Option<Queued> {
    loop {
        if ctx.cancel.is_cancelled() {
            return None;
        }
        let notified = ctx.state.queue_notify.notified();
        {
            let mut q = ctx.state.queue.lock();
            while !q.is_empty() {
                let job = q.remove(0);
                // A re-rank may have re-queued a doc another worker already took.
                if ctx.state.started.lock().insert(job.idx) {
                    return Some(job);
                }
            }
        }
        if ctx.state.dispatch_done.load(Ordering::Acquire) {
            return None;
        }
        tokio::select! {
            _ = ctx.cancel.cancelled() => return None,
            _ = notified => {}
            _ = tokio::time::sleep(Duration::from_millis(400)) => {}
        }
    }
}

fn counts(state: &RunState) -> (usize, usize, usize) {
    (
        state.done.load(Ordering::Relaxed),
        state.failed.load(Ordering::Relaxed),
        state.planned.load(Ordering::Relaxed),
    )
}

fn fail(ctx: &WorkerCtx, doc: Document, error: String) {
    runlog::log(runlog::Event::DownloadFail {
        source: &doc.source,
        title: &doc.title,
        url: &doc.url,
        error: &error,
    });
    ctx.state.failed.fetch_add(1, Ordering::Relaxed);
    let (done, failed, total) = counts(&ctx.state);
    emit(
        ctx.sink.as_ref(),
        EV_DOWNLOAD_FAILED,
        DownloadFailedPayload {
            doc,
            error,
            done,
            failed,
            total,
        },
    );
}

async fn sha256_file(path: PathBuf) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut f = std::fs::File::open(path).ok()?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Some(hex::encode(h.finalize()))
    })
    .await
    .ok()
    .flatten()
}

fn render_markdown(doc: &Document, sources: &[String], ex: &Extracted) -> String {
    let mut fm = front_matter(&[
        ("title", Some(doc.title.clone())),
        (
            "authors",
            (!doc.authors.is_empty()).then(|| doc.authors.join("; ")),
        ),
        ("year", doc.year.clone()),
        ("doi", doc.best_doi()),
        ("venue", doc.venue.clone()),
        ("license", doc.license.clone()),
        ("url", Some(doc.url.clone())),
        ("sources", Some(sources.join(", "))),
        ("pages", ex.pages.map(|p| p.to_string())),
        ("language", ex.lang.clone()),
        ("words", Some(ex.words.to_string())),
        (
            "retrieved",
            Some(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        ),
    ]);
    fm.push_str(&format!("# {}\n\n", doc.title.trim()));
    if let Some(a) = doc.abstract_.as_deref().filter(|a| a.trim().len() > 40) {
        fm.push_str("> **Abstract.** ");
        fm.push_str(a.trim());
        fm.push_str("\n\n");
    }
    fm.push_str(&ex.markdown);
    fm
}

async fn process(ctx: &WorkerCtx, job: Queued) {
    let doc = job.doc;
    emit(
        ctx.sink.as_ref(),
        EV_DOWNLOAD_STARTED,
        DownloadStartedPayload {
            url: doc.url.clone(),
            title: doc.title.clone(),
            source: doc.source.clone(),
        },
    );
    if ctx.cancel.is_cancelled() {
        return;
    }
    let slug = doc.slug();

    // Library-wide reuse: any of this doc's URLs downloaded before?
    let urls = doc.all_urls();
    let reuse_candidates: Vec<String> = {
        let index = Arc::clone(&ctx.index);
        let urls = urls.clone();
        tokio::task::spawn_blocking(move || {
            index
                .lock()
                .as_ref()
                .map(|i| i.lookup_urls(&urls))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    };
    let mut reused: Option<PathBuf> = None;
    for p in reuse_candidates {
        if let Some(dest) = reuse_file(Path::new(&p), &ctx.folder, &slug).await {
            reused = Some(dest);
            break;
        }
    }

    let outcome = if let Some(p) = reused {
        DownloadOutcome::Cached(p)
    } else {
        let sink = Arc::clone(&ctx.sink);
        let url = doc.url.clone();
        let title = doc.title.clone();
        let mut last = Instant::now() - Duration::from_secs(1);
        download(
            &doc,
            &ctx.folder,
            &ctx.client,
            &ctx.limiter,
            &ctx.cancel,
            move |ev| {
                if !ev.force
                    && last.elapsed() < Duration::from_millis(200)
                    && (ev.total == 0 || ev.downloaded < ev.total)
                {
                    return;
                }
                last = Instant::now();
                emit(
                    sink.as_ref(),
                    EV_DOWNLOAD_PROGRESS,
                    DownloadProgressPayload {
                        url: url.clone(),
                        title: title.clone(),
                        downloaded: ev.downloaded,
                        total: ev.total,
                    },
                );
            },
        )
        .await
    };

    let (path, cached) = match outcome {
        DownloadOutcome::Saved(p) => (p, false),
        DownloadOutcome::Cached(p) => (p, true),
        DownloadOutcome::Cancelled => return,
        DownloadOutcome::Failed(e) => return fail(ctx, doc, e),
    };
    let bytes = tokio::fs::metadata(&path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    // Identical file already saved this run under another title/URL?
    let sha = sha256_file(path.clone()).await;
    if let Some(h) = &sha {
        let dup_of = {
            let mut hashes = ctx.state.hashes.lock();
            match hashes.get(h) {
                Some(t) => Some(t.clone()),
                None => {
                    hashes.insert(h.clone(), doc.title.clone());
                    None
                }
            }
        };
        if let Some(other) = dup_of {
            if !cached {
                let _ = tokio::fs::remove_file(&path).await;
            }
            return fail(ctx, doc, format!("Same file as “{other}”, already saved."));
        }
        let index = Arc::clone(&ctx.index);
        let rec_urls = urls.clone();
        let h = h.clone();
        let abs = path.to_string_lossy().to_string();
        tokio::spawn(tokio::task::spawn_blocking(move || {
            if let Some(i) = index.lock().as_ref() {
                let _ = i.record(&rec_urls, &h, &abs);
            }
        }));
    }
    runlog::log(runlog::Event::DownloadOk {
        source: &doc.source,
        title: &doc.title,
        url: &doc.url,
        local_path: &path.to_string_lossy(),
        bytes,
    });

    // ---- Read: extract Markdown + chunks (CPU-bound, own semaphore) ----
    let mut md_rel: Option<String> = None;
    let mut extract_error: Option<String> = None;
    let mut extracted: Option<Extracted> = None;
    if !ctx.cancel.is_cancelled() {
        if let Ok(_permit) = ctx.extract_sem.clone().acquire_owned().await {
            let p = path.clone();
            let title = doc.title.clone();
            let res = tokio::task::spawn_blocking(move || extract(&p, &title))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("text extraction crashed: {e}")));
            match res {
                Ok(ex) => {
                    let md_path = ctx.text_dir.join(format!("{slug}.md"));
                    let body = render_markdown(&doc, &job.sources, &ex);
                    if tokio::fs::write(&md_path, body).await.is_ok() {
                        md_rel = md_path
                            .strip_prefix(&ctx.folder)
                            .ok()
                            .map(|r| r.to_string_lossy().to_string());
                    }
                    extracted = Some(ex);
                }
                Err(e) => extract_error = Some(e.to_string()),
            }
        }
    }

    let local_path = path
        .strip_prefix(&ctx.folder)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let rec = DocRecord {
        title: doc.title.clone(),
        url: doc.url.clone(),
        source: doc.source.clone(),
        sources: job.sources.clone(),
        authors: doc.authors.clone(),
        year: doc.year.clone(),
        abstract_: doc.abstract_.clone(),
        doi: doc.best_doi(),
        venue: doc.venue.clone(),
        license: doc.license.clone(),
        cited_by: doc.cited_by,
        score: Some(job.score),
        local_path: local_path.clone(),
        text_path: md_rel.clone(),
        md_path: md_rel.clone(),
        extract_error: extract_error.clone(),
        size_bytes: bytes,
        sha256: sha,
        pages: extracted.as_ref().and_then(|e| e.pages),
        lang: extracted.as_ref().and_then(|e| e.lang.clone()),
        words: extracted.as_ref().map(|e| e.words as u64),
        tokens: extracted.as_ref().map(|e| e.tokens as u64),
    };
    let chunks = extracted.map(|e| e.chunks).unwrap_or_default();
    let (reply_tx, reply_rx) = oneshot::channel();
    let db_err = if ctx.writer.send((rec, chunks, reply_tx)).is_err() {
        Some("database writer stopped".to_string())
    } else {
        match reply_rx.await {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some("database writer stopped".to_string()),
        }
    };
    if let Some(err) = &db_err {
        runlog::log(runlog::Event::DbError {
            title: &doc.title,
            url: &doc.url,
            error: err,
        });
    }
    ctx.state.done.fetch_add(1, Ordering::Relaxed);
    let (done, failed, total) = counts(&ctx.state);
    emit(
        ctx.sink.as_ref(),
        EV_DOWNLOAD_DONE,
        DownloadDonePayload {
            doc,
            local_path,
            absolute_path: path.to_string_lossy().to_string(),
            text_path: md_rel,
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

async fn worker(ctx: Arc<WorkerCtx>) {
    while let Some(job) = next_job(&ctx).await {
        let title = job.doc.title.clone();
        let doc = job.doc.clone();
        // Isolate panics per document: a crash counts as a failure, the run
        // continues, and done + failed still adds up.
        let ctx2 = Arc::clone(&ctx);
        let res = tokio::spawn(async move { process(&ctx2, job).await }).await;
        if let Err(e) = res {
            if e.is_panic() {
                tracing::error!("download task panicked for {title}: {e}");
                fail(
                    &ctx,
                    doc,
                    "Something went wrong while saving this document.".into(),
                );
            }
        }
    }
}

fn spawn_workers(ctx: Arc<WorkerCtx>, n: usize) -> JoinSet<()> {
    let mut set = JoinSet::new();
    for _ in 0..n {
        set.spawn(worker(Arc::clone(&ctx)));
    }
    set
}

struct Library {
    folder: PathBuf,
    text_dir: PathBuf,
    db_path: PathBuf,
    run_id: i64,
}

async fn open_library(folder: PathBuf, query: &str) -> anyhow::Result<Library> {
    tokio::fs::create_dir_all(&folder).await?;
    sweep_stale_parts(&folder).await;
    let text_dir = folder.join("_text");
    tokio::fs::create_dir_all(&text_dir).await?;
    let db_path = folder.join("library.db");
    let run_id = {
        let mgr = DbManager::new(&db_path).map_err(|e| anyhow::anyhow!("DB init failed: {e}"))?;
        mgr.insert_run(query, &folder.to_string_lossy())
            .map_err(|e| anyhow::anyhow!("failed to insert run: {e}"))?
    };
    Ok(Library {
        folder,
        text_dir,
        db_path,
        run_id,
    })
}

fn open_index(root: Option<&Path>) -> Arc<Mutex<Option<LibraryIndex>>> {
    let idx = root.and_then(|r| match LibraryIndex::open(r) {
        Ok(i) => Some(i),
        Err(e) => {
            tracing::warn!("library index unavailable: {e}");
            None
        }
    });
    Arc::new(Mutex::new(idx))
}

fn worker_ctx(
    sink: &Arc<dyn Sink>,
    state: &Arc<RunState>,
    cancel: &CancellationToken,
    lib: &Library,
    index: Arc<Mutex<Option<LibraryIndex>>>,
) -> Arc<WorkerCtx> {
    Arc::new(WorkerCtx {
        sink: Arc::clone(sink),
        state: Arc::clone(state),
        cancel: cancel.clone(),
        client: Arc::new(make_download_client()),
        limiter: HostLimiter::new(),
        folder: lib.folder.clone(),
        text_dir: lib.text_dir.clone(),
        writer: spawn_writer(lib.db_path.clone(), lib.run_id),
        index,
        extract_sem: Arc::new(Semaphore::new((num_cpus::get() / 2).clamp(1, 4))),
    })
}

fn finish(sink: &dyn Sink, state: &RunState, lib: &Library, cancel: &CancellationToken) {
    let (done, failed, planned) = counts(state);
    let total = planned.max(done + failed);
    emit_stage(
        sink,
        "download",
        "done",
        Some(done as u64),
        Some(total as u64),
        Some(format!("{done} saved · {failed} failed")),
    );
    emit_stage(
        sink,
        "extract",
        "done",
        Some(done as u64),
        Some(total as u64),
        None,
    );
    let folder = lib.folder.to_string_lossy().to_string();
    runlog::log(runlog::Event::RunComplete {
        done,
        failed,
        total,
        folder: &folder,
    });
    emit(
        sink,
        if cancel.is_cancelled() {
            EV_CANCELLED
        } else {
            EV_COMPLETE
        },
        CompletePayload {
            done,
            failed,
            total,
            folder,
            manifest: lib.db_path.to_string_lossy().to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Run a full search. `app` is only needed by sources that report their own
/// health (meta-search); headless callers pass `None`.
pub async fn run_pipeline(
    sink: Arc<dyn Sink>,
    app: Option<AppHandle>,
    req: RunRequest,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let client = Arc::new(make_client());
    let base = expand_query_broad(&req.query);
    runlog::log(runlog::Event::RunStart {
        query: &req.query,
        sub_queries: &base,
        sources: &req.sources,
    });
    emit(
        sink.as_ref(),
        EV_KEYWORDS,
        KeywordsPayload {
            query: req.query.clone(),
            sub_queries: base.clone(),
        },
    );

    let root = PathBuf::from(&req.out_dir);
    let lib = open_library(root.join(safe_folder(&req.query)), &req.query).await?;
    let index = open_index(Some(&root));

    // The meta-search aggregator already covers the individual web scrapers.
    const META_COVERED: &[&str] = &[
        "web",
        "brave",
        "bing",
        "mojeek",
        "marginalia",
        "startpage",
        "searxng",
    ];
    let sources: Vec<String> = if req.sources.iter().any(|s| s == "meta_search") {
        req.sources
            .iter()
            .filter(|s| !META_COVERED.contains(&s.as_str()))
            .cloned()
            .collect()
    } else {
        req.sources.clone()
    };

    let keywords = parse_query(&req.query);
    let qmodel = QueryModel::from_keywords(if keywords.is_empty() {
        std::slice::from_ref(&req.query)
    } else {
        &keywords
    });
    let state = Arc::new(RunState::new(qmodel, sources.len()));
    let source_sems: HashMap<String, Arc<Semaphore>> = sources
        .iter()
        .map(|s| (s.clone(), Arc::new(Semaphore::new(source_concurrency(s)))))
        .collect();

    emit_stage(
        sink.as_ref(),
        "discovery",
        "started",
        None,
        None,
        Some(format!(
            "{} sources × {} searches",
            sources.len(),
            base.len()
        )),
    );
    emit_stage(sink.as_ref(), "download", "started", None, None, None);
    emit_stage(sink.as_ref(), "extract", "started", None, None, None);

    // Workers start now and idle until the dispatcher queues work.
    let ctx = worker_ctx(&sink, &state, &cancel, &lib, index);
    let mut workers = spawn_workers(Arc::clone(&ctx), DOWNLOAD_WORKERS);

    // Dispatcher: periodic re-rank → best-first queue.
    let dispatcher = {
        let state = Arc::clone(&state);
        let sink = Arc::clone(&sink);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let t0 = Instant::now();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(RERANK_EVERY) => {}
                }
                if state.discovery_done.load(Ordering::Acquire) {
                    break;
                }
                let n = state.dedup.lock().len();
                if n >= WARMUP_MIN || (n > 0 && t0.elapsed() >= WARMUP_TIME) {
                    let _ = dispatch_pass(&state, sink.as_ref(), false);
                }
            }
        })
    };

    // ---- Wave 1 ----
    discover_wave(
        &sink,
        &app,
        &client,
        &cancel,
        &state,
        &base,
        &sources,
        &req,
        WAVE1_DEADLINE,
        &source_sems,
    )
    .await;

    // ---- Feedback: citation graph + wave 2 (only if needed) ----
    if !cancel.is_cancelled() {
        let ranked = state.rank_now();
        let kept = ranked
            .iter()
            .filter(|(_, r)| r.reject_reason.is_none())
            .count();

        let top_dois: Vec<(String, String)> = ranked
            .iter()
            .filter(|(_, r)| r.reject_reason.is_none())
            .filter_map(|(_, r)| r.doc.doc.best_doi().map(|d| (r.doc.doc.url.clone(), d)))
            .take(30)
            .collect();
        let citation_task = {
            let state = Arc::clone(&state);
            let client = Arc::clone(&client);
            tokio::spawn(async move {
                let boosts = citation_boosts(client, top_dois, CITATION_BUDGET).await;
                if !boosts.is_empty() {
                    *state.boosts.lock() = boosts;
                }
            })
        };

        let ranked_only: Vec<RankedDoc> = ranked.into_iter().map(|(_, r)| r).collect();
        let q = state.query.lock().clone();
        let expansion = expansion_terms(&ranked_only, &q, 12, 8);
        let wave2 = if kept < ENOUGH_KEPT {
            feedback_subqueries(&keywords, &expansion)
        } else {
            Vec::new()
        };
        if !expansion.is_empty() {
            let mut qm = state.query.lock();
            *qm = qm.clone().with_expansion(&expansion);
        }
        if !wave2.is_empty() && state.dedup.lock().len() < MAX_CANDIDATES {
            tracing::info!("feedback wave: {wave2:?}");
            let mut all = base.clone();
            all.extend(wave2.iter().cloned());
            emit(
                sink.as_ref(),
                EV_KEYWORDS,
                KeywordsPayload {
                    query: req.query.clone(),
                    sub_queries: all,
                },
            );
            discover_wave(
                &sink,
                &app,
                &client,
                &cancel,
                &state,
                &wave2,
                &sources,
                &req,
                WAVE2_DEADLINE,
                &source_sems,
            )
            .await;
        }
        // Let the citation pass land (it is bounded by its own budget).
        let _ = tokio::time::timeout(CITATION_BUDGET, citation_task).await;
    }

    state.discovery_done.store(true, Ordering::Release);
    let _ = dispatcher.await;
    let total_found = state.dedup.lock().len();
    emit(
        sink.as_ref(),
        EV_FOUND_TOTAL,
        FoundTotalPayload { count: total_found },
    );
    emit_stage(
        sink.as_ref(),
        "discovery",
        "done",
        Some(total_found as u64),
        None,
        Some(format!("{total_found} unique candidates")),
    );

    if !cancel.is_cancelled() {
        let ranked = dispatch_pass(&state, sink.as_ref(), true);
        emit_candidates(sink.as_ref(), &ranked);
    }
    state.dispatch_done.store(true, Ordering::Release);
    state.queue_notify.notify_waiters();

    while workers.join_next().await.is_some() {}
    finish(sink.as_ref(), &state, &lib, &cancel);
    Ok(())
}

/// Re-download a fixed list of documents into an existing library (the
/// "Retry failed" action) — same workers, no discovery.
pub async fn run_retry_pipeline(
    sink: Arc<dyn Sink>,
    folder: PathBuf,
    query: String,
    docs: Vec<Document>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let root = folder.parent().map(Path::to_path_buf);
    let lib = open_library(folder, &query).await?;
    let state = Arc::new(RunState::new(QueryModel::default(), 1));
    let n = docs.len().min(MAX_DOWNLOADS);
    *state.queue.lock() = docs
        .into_iter()
        .take(n)
        .enumerate()
        .map(|(i, d)| Queued {
            idx: i,
            score: 0.0,
            sources: vec![d.source.clone()],
            doc: d,
        })
        .collect();
    state.planned.store(n, Ordering::Relaxed);
    state.discovery_done.store(true, Ordering::Release);
    state.dispatch_done.store(true, Ordering::Release);
    emit(sink.as_ref(), EV_PLAN, PlanPayload { planned: n });
    emit_stage(
        sink.as_ref(),
        "download",
        "started",
        None,
        Some(n as u64),
        Some(format!("{n} to retry")),
    );
    emit_stage(
        sink.as_ref(),
        "extract",
        "started",
        None,
        Some(n as u64),
        None,
    );
    let ctx = worker_ctx(&sink, &state, &cancel, &lib, open_index(root.as_deref()));
    let mut workers = spawn_workers(ctx, DOWNLOAD_WORKERS.min(n.max(1)));
    while workers.join_next().await.is_some() {}
    finish(sink.as_ref(), &state, &lib, &cancel);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    pub struct Recorder(pub Mutex<Vec<(String, serde_json::Value)>>);
    impl Sink for Recorder {
        fn emit_value(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().push((event.to_string(), payload));
        }
    }

    fn doc(title: &str, url: &str) -> Document {
        Document {
            title: title.into(),
            url: url.into(),
            source: "openalex".into(),
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_queues_best_first_and_respects_early_share() {
        let state = RunState::new(
            QueryModel::from_keywords(&["solar".into(), "panels".into()]),
            2,
        );
        {
            let mut d = state.dedup.lock();
            d.add(doc("Solar panels efficiency", "https://a/1"), "openalex", 1);
            d.add(doc("Solar panels on roofs", "https://a/2"), "openalex", 2);
            d.add(
                doc("Solar panels and solar panels", "https://a/3"),
                "arxiv",
                1,
            );
            d.add(doc("Knitting patterns", "https://a/4"), "openalex", 3);
        }
        let rec = Recorder::default();
        let ranked = dispatch_pass(&state, &rec, false);
        assert_eq!(ranked.len(), 4);
        let q = state.queue.lock();
        // 3 kept × 0.6 → 2 dispatched early; the off-topic doc never.
        assert_eq!(q.len(), 2);
        assert!(q.iter().all(|j| !j.doc.title.contains("Knitting")));
        assert!(q[0].score >= q[1].score);
        drop(q);
        let _ = dispatch_pass(&state, &rec, true);
        assert_eq!(state.queue.lock().len(), 3);
        assert!(rec.0.lock().iter().any(|(e, _)| e == EV_PLAN));
    }

    #[test]
    fn fair_share_quota_scales_with_sources() {
        let s = RunState::new(QueryModel::default(), 6);
        assert_eq!(s.source_quota, 625);
        let s1 = RunState::new(QueryModel::default(), 1);
        assert!(s1.source_quota >= MAX_CANDIDATES);
    }

    #[tokio::test]
    async fn retry_pipeline_completes_and_counts_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let rec: Arc<Recorder> = Arc::new(Recorder::default());
        let sink: Arc<dyn Sink> = rec.clone();
        // Unroutable docs: every route fails fast (SSRF guard rejects them).
        let docs = vec![
            doc("A", "http://127.0.0.1:9/a.pdf"),
            doc("B", "http://127.0.0.1:9/b.pdf"),
        ];
        run_retry_pipeline(
            sink,
            tmp.path().join("lib"),
            "q".into(),
            docs,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let events = rec.0.lock();
        let complete = events.iter().find(|(e, _)| e == EV_COMPLETE).unwrap();
        assert_eq!(complete.1["failed"], 2);
        assert_eq!(complete.1["done"], 0);
    }
}
