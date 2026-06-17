//! Semantic embeddings via `fastembed` (BGE-Small-EN-v1.5, 384-dim, int8 ONNX).
//!
//! fastembed/ort can abort the process natively (not a catchable Rust panic)
//! during ONNX Runtime init on some platforms (macOS). So all fastembed/ort
//! work runs in a **child process** (see [`super::embed_worker`]); this module
//! is the *client* that spawns it, talks JSON over its stdin/stdout, and — when
//! the worker dies — degrades gracefully to lexical ranking instead of letting
//! the whole app crash. The model is downloaded by the worker on first use and
//! cached under `<app_data>/models/fastembed/`, fetched once then loaded offline.

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel as FastEmbedModel, InitOptions, TextEmbedding};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::embed_worker::{WorkerRequest, WorkerResponse, WORKER_ARG};

/// Owns the in-process fastembed model. Constructed ONLY inside the worker
/// child (the parent never touches ort), so a native abort can't reach the UI.
pub struct EmbeddingModel {
    inner: TextEmbedding,
}

impl EmbeddingModel {
    /// Load BGE-Small-EN-v1.5 via fastembed, downloading + caching it on first
    /// use. Takes a cache dir directly (the worker has no `AppHandle`).
    pub fn init_with_cache_dir(cache_dir: &Path) -> Result<Self> {
        let _ = std::fs::create_dir_all(cache_dir);
        let inner = TextEmbedding::try_new(
            InitOptions::new(FastEmbedModel::BGESmallENV15Q)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(false),
        )
        .context("fastembed BGE-Small init failed (first-run download needs internet)")?;
        Ok(Self { inner })
    }

    /// Embed a batch of texts. Returns one `Vec<f32>` per input. Called only in
    /// the worker process.
    pub fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.inner
            .embed(refs, None)
            .context("embedding inference failed")
    }
}

/// Where fastembed caches the model — under the app data dir so it survives
/// across runs and is fetched only once. The parent computes this and passes it
/// to the worker.
fn cache_dir(app: &AppHandle) -> Result<std::path::PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .context("could not resolve app data directory")?
        .join("models")
        .join("fastembed");
    let _ = std::fs::create_dir_all(&dir);
    Ok(dir)
}

/// Cosine similarity. fastembed returns L2-normalized vectors, so this is
/// effectively a dot product, but we divide by the norm product defensively.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = (na.sqrt() * nb.sqrt()).max(1e-9);
    dot / denom
}

// =============================================================================
// Worker client — manages the child process and the JSON request/response IPC.
// =============================================================================

/// How long to wait for a worker response before treating an alive-but-silent
/// worker as a crash. `Warm` includes the first-run model download, so it gets a
/// generous budget; `Embed` is pure inference on an already-loaded model and is
/// quick. Bounding the wait is what turns a *hang* (ort deadlock, a black-hole
/// connection during the first download) into the same graceful degradation to
/// lexical ranking that a native *abort* already gets — instead of an unbounded
/// blocked thread pinning the client mutex for the session.
const WARM_TIMEOUT: Duration = Duration::from_secs(300);
// Pure inference on an already-loaded model — embedding a few hundred short
// texts is a matter of seconds even on CPU. Kept well under the old 120s so a
// hung worker can't stall the ranking phase (which downloads wait on) for two
// minutes before degrading to lexical ranking.
const EMBED_TIMEOUT: Duration = Duration::from_secs(45);

fn request_timeout(req: &WorkerRequest) -> Duration {
    match req {
        WorkerRequest::Warm => WARM_TIMEOUT,
        WorkerRequest::Embed { .. } => EMBED_TIMEOUT,
    }
}

struct WorkerClient {
    child: Child,
    stdin: ChildStdin,
    /// Response lines drained off the worker's stdout by `stdout_thread`. A
    /// disconnected channel means stdout hit EOF — the worker died.
    responses: Receiver<String>,
    /// Last ~64 stderr lines (phase markers) for crash forensics.
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl WorkerClient {
    /// Send one request and wait up to `timeout` for its response. `Err(())`
    /// means the worker is unusable — it died (channel disconnected / write
    /// failed), hung (no response within `timeout`), or sent an unparseable
    /// line. The caller treats all of these identically: reap it, latch
    /// `UNAVAILABLE`, and degrade to lexical ranking.
    fn request(
        &mut self,
        req: &WorkerRequest,
        timeout: Duration,
    ) -> std::result::Result<WorkerResponse, ()> {
        let line = serde_json::to_string(req).map_err(|_| ())?;
        if writeln!(self.stdin, "{line}").is_err() || self.stdin.flush().is_err() {
            return Err(());
        }
        match self.responses.recv_timeout(timeout) {
            Ok(resp) => serde_json::from_str(&resp).map_err(|_| ()),
            // Timeout (worker hung) or Disconnected (stdout closed = worker
            // died) — both mean the worker can no longer be trusted.
            Err(_) => Err(()),
        }
    }
}

static CLIENT: OnceLock<Mutex<Option<WorkerClient>>> = OnceLock::new();
/// Set once the worker has successfully loaded the model this session.
static LOADED: AtomicBool = AtomicBool::new(false);
/// Latched when the worker dies (likely native abort). Stops respawn loops;
/// cleared by an explicit user warm or `reset_embedding_model`.
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);
/// Dedups concurrent background warms.
static WARMING: AtomicBool = AtomicBool::new(false);

fn client_lock() -> &'static Mutex<Option<WorkerClient>> {
    CLIENT.get_or_init(|| Mutex::new(None))
}

fn spawn_worker(cache_dir: &Path) -> Result<WorkerClient> {
    let exe = std::env::current_exe().context("resolve current_exe for embedding worker")?;
    let mut child = Command::new(exe)
        .arg(WORKER_ARG)
        .arg(cache_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn embedding worker")?;
    let stdin = child.stdin.take().context("embedding worker stdin")?;
    let stdout = child.stdout.take().context("embedding worker stdout")?;
    let stderr = child.stderr.take().context("embedding worker stderr")?;

    // Drain stdout on a dedicated thread, forwarding whole response lines over a
    // channel. This lets `request` apply a `recv_timeout` (so an alive-but-silent
    // worker can't block the caller — and the held client mutex — forever) and
    // keeps the worker from ever stalling on a full stdout pipe.
    let (tx, responses) = channel::<String>();
    let stdout_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if tx.send(line).is_err() {
                break; // receiver gone (client dropped) — stop draining
            }
        }
        // Loop end drops `tx`; a waiting `recv_timeout` then sees Disconnected,
        // i.e. the worker's stdout closed (it died).
    });

    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(64)));
    let tail = stderr_tail.clone();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::result::Result::ok)
        {
            let mut g = tail.lock().unwrap_or_else(|e| e.into_inner());
            if g.len() == 64 {
                g.pop_front();
            }
            g.push_back(line);
        }
    });

    Ok(WorkerClient {
        child,
        stdin,
        responses,
        stderr_tail,
        stdout_thread: Some(stdout_thread),
        stderr_thread: Some(stderr_thread),
    })
}

/// Send a request to the worker, spawning it on first use. On a worker crash,
/// latch `UNAVAILABLE`, record diagnostics to the run log, and return `Err`.
fn worker_request(cache_dir: &Path, req: &WorkerRequest) -> Result<WorkerResponse> {
    if UNAVAILABLE.load(Ordering::SeqCst) {
        anyhow::bail!("embedding worker previously crashed; semantic rerank disabled this session");
    }
    let mut guard = client_lock().lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(spawn_worker(cache_dir)?);
    }
    let client = guard.as_mut().expect("spawned just above");
    match client.request(req, request_timeout(req)) {
        Ok(WorkerResponse::Error { message }) => {
            // Recoverable worker-side failure — keep the worker alive for retry.
            anyhow::bail!("embedding failed: {message}")
        }
        Ok(resp) => Ok(resp),
        Err(()) => {
            // Worker died (native abort) or hung past its deadline. Capture + latch.
            let dead = guard.take();
            UNAVAILABLE.store(true, Ordering::SeqCst);
            LOADED.store(false, Ordering::SeqCst);
            drop(guard);
            if let Some(c) = dead {
                capture_worker_failure(c);
            }
            anyhow::bail!("embedding worker crashed; semantic rerank disabled this session")
        }
    }
}

/// Reap a dead worker and log where/why it died to the run log (the recoverable
/// equivalent of the macOS `.ips` crash report).
fn capture_worker_failure(mut client: WorkerClient) {
    // The worker either died on its own (native abort) or hung past its deadline;
    // `kill` is a harmless no-op on an already-exited child and, for a hang,
    // guarantees the pipes close so the drain threads below can finish.
    let _ = client.child.kill();
    let exit = match client.child.wait() {
        Ok(status) => exit_desc(status),
        Err(e) => format!("wait failed: {e}"),
    };
    // Child is reaped → its stdout/stderr pipes close → the drain threads finish.
    if let Some(t) = client.stdout_thread.take() {
        let _ = t.join();
    }
    if let Some(t) = client.stderr_thread.take() {
        let _ = t.join();
    }
    let tail = client.stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
    let last_phase = tail
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("phase:"))
        .unwrap_or("unknown")
        .to_string();
    let recent: Vec<String> = tail.iter().rev().take(8).cloned().collect();
    let stderr_tail = recent.into_iter().rev().collect::<Vec<_>>().join(" | ");
    tracing::warn!("embedding worker died: phase={last_phase} exit={exit} stderr=[{stderr_tail}]");
    crate::engine::runlog::log(crate::engine::runlog::Event::EmbeddingWorkerFailed {
        phase: &last_phase,
        exit: &exit,
        stderr_tail: &stderr_tail,
    });
}

#[cfg(unix)]
fn exit_desc(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(sig) = status.signal() {
        return format!("killed by signal {sig}");
    }
    format!("{status}")
}

#[cfg(not(unix))]
fn exit_desc(status: std::process::ExitStatus) -> String {
    format!("{status}")
}

fn emit_status(app: &AppHandle, status: &str, detail: &str) {
    use tauri::Emitter;
    let _ = app.emit(
        crate::events::EV_MODEL_STATUS,
        crate::events::ModelStatusPayload {
            model_id: "bge-small-en-v1.5".to_string(),
            status: status.to_string(),
            detail: Some(detail.to_string()),
        },
    );
}

// =============================================================================
// Public API (signatures preserved for callers in commands.rs / orchestrator).
// =============================================================================

/// Whether the embedding model is loaded and usable this session.
pub fn is_loaded() -> bool {
    LOADED.load(Ordering::SeqCst) && !UNAVAILABLE.load(Ordering::SeqCst)
}

/// Whether the model is already cached on disk (loads without a network fetch).
/// Best-effort scan of the fastembed cache dir for a `.onnx` file; does not
/// touch ort, so it's safe to call in the UI process.
pub fn is_downloaded(app: &AppHandle) -> bool {
    let Ok(dir) = cache_dir(app) else {
        return false;
    };
    contains_onnx(&dir)
}

fn contains_onnx(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if contains_onnx(&path) {
                return true;
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("onnx") {
            return true;
        }
    }
    false
}

/// Drop the worker so the next call re-spawns/re-loads it. Clears the crash
/// latch so the user can retry after `reset_ai_state`.
pub fn reset_embedding_model() {
    // Take the worker OUT of the lock, then DROP the guard before the blocking
    // kill+wait — never hold `client_lock` across a syscall, or a concurrent
    // `worker_request` could stall acquiring it. `reset_ai_state` offloads this
    // whole fn to `spawn_blocking` so the wait doesn't pin an async runtime thread.
    let taken = {
        let mut guard = client_lock().lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    };
    if let Some(mut c) = taken {
        let _ = c.child.kill();
        let _ = c.child.wait();
    }
    LOADED.store(false, Ordering::SeqCst);
    UNAVAILABLE.store(false, Ordering::SeqCst);
    tracing::info!("embedding model reset");
}

/// User-initiated warm (Settings "Download now"). Clears the crash latch so a
/// previously-failed load can be retried, then loads in the background.
pub fn warm_in_background(app: AppHandle) {
    UNAVAILABLE.store(false, Ordering::SeqCst);
    warm_inner(app);
}

/// Implicit warm triggered by a search. Skips entirely if the worker already
/// crashed this session, so a search can never trigger a crash loop.
pub fn warm_in_background_implicit(app: AppHandle) {
    if UNAVAILABLE.load(Ordering::SeqCst) {
        return;
    }
    warm_inner(app);
}

fn warm_inner(app: AppHandle) {
    if LOADED.load(Ordering::SeqCst) || WARMING.swap(true, Ordering::SeqCst) {
        return;
    }
    // MUST use Tauri's runtime-agnostic spawner, NOT `tokio::task::spawn_blocking`.
    // This is reached from the SYNCHRONOUS `warm_embedding` #[tauri::command]
    // (the Settings "Download" button), which Tauri runs on the main thread with
    // NO tokio runtime entered. `tokio::task::spawn_blocking` there panics ("must
    // be called from the context of a Tokio runtime"), and that panic unwinding
    // back across the C++/WebKit FFI boundary aborts the whole app (observed as a
    // SIGABRT on macOS). `tauri::async_runtime::spawn_blocking` uses Tauri's global
    // runtime handle, so it works from the main thread AND from the in-search
    // (already-in-runtime) caller.
    tauri::async_runtime::spawn_blocking(move || {
        // Release the WARMING dedup flag on EVERY exit, including a panic in the
        // warm task — a manual reset at the end would be skipped on unwind,
        // permanently disabling warm-up (and silently the semantic rerank) for
        // the rest of the session.
        let _warming_guard = WarmingResetGuard;
        let cache = match cache_dir(&app) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("embedding warm: cache dir: {e}");
                return;
            }
        };
        match worker_request(&cache, &WorkerRequest::Warm) {
            Ok(WorkerResponse::Warmed) => {
                LOADED.store(true, Ordering::SeqCst);
                tracing::info!("embedding model warmed and ready");
                emit_status(&app, "embedding", "ready");
            }
            Ok(_) => tracing::warn!("embedding warm: unexpected worker response"),
            Err(e) => {
                tracing::warn!("embedding model warm failed: {e}");
                emit_status(&app, "embedding_failed", &e.to_string());
            }
        }
    });
}

/// Resets the `WARMING` dedup flag when dropped (normal return OR panic), so a
/// crash in the warm task can't latch it `true` for the session.
struct WarmingResetGuard;
impl Drop for WarmingResetGuard {
    fn drop(&mut self) {
        WARMING.store(false, Ordering::SeqCst);
    }
}

/// Semantic rerank: embed the query + each candidate's (title + abstract) in the
/// worker, blend cosine similarity with the Tier-1 score (`0.6*tier1 +
/// 0.4*semantic`), and re-sort. Runs on a blocking thread (caller uses
/// `spawn_blocking`). Only the first `top_k` candidates are re-embedded; the
/// rest keep their Tier-1 score. On worker crash this returns `Err` and the
/// orchestrator falls back to the existing ranking.
pub fn rerank_blocking(
    app: &AppHandle,
    query: &str,
    candidates: &mut [super::super::engine::ranking::RankedDoc],
    top_k: usize,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let cache = cache_dir(app)?;
    let limit = top_k.min(candidates.len());

    // One batch: [query, doc_0, doc_1, ...].
    let mut texts: Vec<String> = Vec::with_capacity(limit + 1);
    texts.push(query.to_string());
    for c in candidates.iter().take(limit) {
        let abstract_ = c.doc.doc.abstract_.as_deref().unwrap_or("");
        texts.push(format!("{} {}", c.doc.doc.title, abstract_));
    }

    let WorkerResponse::Embeddings { vectors } =
        worker_request(&cache, &WorkerRequest::Embed { texts })?
    else {
        anyhow::bail!("embedding worker returned an unexpected response");
    };
    if vectors.is_empty() {
        anyhow::bail!("embedding worker returned no vectors");
    }
    LOADED.store(true, Ordering::SeqCst);

    let query_emb = &vectors[0];
    let doc_embs = &vectors[1..];
    // The reranked head == the candidates we actually embedded (the top `limit`).
    let head = doc_embs.len().min(candidates.len());

    // Normalize the head's lexical scores to [0,1] BEFORE blending. Raw lexical
    // scores are an unbounded product `(tfidf+eps) * (rrf+eps) * authority` that
    // realistically lands in ~0.001–0.5, while cosine `sim` is already [0,1].
    // Blending them directly let the `0.4*sim` term dominate `0.6*lexical`,
    // collapsing the intended 60/40 weighting to ~95% semantic. Min-max over the
    // head makes the weights meaningful again.
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for c in candidates.iter().take(head) {
        lo = lo.min(c.score);
        hi = hi.max(c.score);
    }
    let range = hi - lo;
    for (i, doc_emb) in doc_embs.iter().enumerate().take(head) {
        let sim = cosine(query_emb, doc_emb).clamp(0.0, 1.0);
        let norm_lex = if range > f32::EPSILON {
            (candidates[i].score - lo) / range
        } else {
            1.0 // all head scores equal → lexical carries no signal; rank by sim
        };
        candidates[i].score = 0.6 * norm_lex + 0.4 * sim;
    }

    // Sort ONLY the reranked head. The un-reranked tail keeps its raw lexical
    // scores and stays positionally below the head (it was already the lower-
    // lexical remainder), so a normalized head score is never compared against an
    // unscaled tail score.
    candidates[..head].sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves fastembed can download + load BGE-Small and produce a 384-dim
    /// vector. Hits the network (~33 MB), so it's `#[ignore]`d by default:
    /// `cargo test --no-default-features --features=custom-protocol,ai-embeddings -- --ignored`
    #[ignore]
    #[test]
    fn fastembed_downloads_and_embeds() {
        let tmp = std::env::temp_dir().join("df-fastembed-test");
        let mut model = EmbeddingModel::init_with_cache_dir(&tmp).expect("init/download failed");
        let out = model
            .embed_batch(&["the quick brown fox".to_string()])
            .expect("embedding failed");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 384, "BGE-Small-EN-v1.5 is 384-dimensional");
    }
}
