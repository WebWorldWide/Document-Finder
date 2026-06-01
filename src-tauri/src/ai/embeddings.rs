//! Semantic embeddings via `fastembed`.
//!
//! Uses BGE-Small-EN-v1.5 (384-dim, int8-quantized ONNX). fastembed downloads
//! the model from Hugging Face on first use and caches it under the app data
//! dir (`<app_data>/models/fastembed/`), so it's fetched once and then loaded
//! offline on every subsequent run — nothing is bundled into the installer.
//! (Earlier builds tried to bundle the ONNX files as Tauri resources, but they
//! were never wired into `bundle.resources`, so the model never loaded.)
//!
//! Inference is synchronous (ort is blocking); we wrap calls in
//! `tokio::task::spawn_blocking` so the orchestrator's async runtime
//! stays responsive while reranking runs.

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel as FastEmbedModel, InitOptions, TextEmbedding};
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub struct EmbeddingModel {
    inner: TextEmbedding,
}

/// Where fastembed caches the downloaded model — under the app data dir so it
/// survives across runs and is fetched only once.
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

impl EmbeddingModel {
    /// Load BGE-Small-EN-v1.5 via fastembed, downloading + caching it on first
    /// use (subsequent loads are offline from the cache). Only happens once per
    /// process via the singleton below.
    pub fn init(app: &AppHandle) -> Result<Self> {
        let inner = TextEmbedding::try_new(
            InitOptions::new(FastEmbedModel::BGESmallENV15Q)
                .with_cache_dir(cache_dir(app)?)
                .with_show_download_progress(false),
        )
        .context("fastembed BGE-Small init failed (first-run download needs internet)")?;

        Ok(Self { inner })
    }

    /// Embed a batch of texts. Returns one Vec<f32> per input. Internal
    /// batching is handled by fastembed (default size 256, plenty).
    /// Requires `&mut self` because the underlying ONNX session is stateful.
    pub fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.inner
            .embed(refs, None)
            .context("embedding inference failed")
    }
}

/// Cosine similarity between two unit-length-ish vectors. fastembed
/// already returns L2-normalized vectors, so this is effectively a dot
/// product, but we still divide by the norm product to be defensive.
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
// Singleton — load once per process, resettable on demand.
//
// Uses parking_lot::Mutex (no poisoning) so a panic during ONNX inference
// releases the lock cleanly rather than permanently poisoning it. The outer
// RwLock<Option<...>> allows reset_embedding_model() to drop the model and
// force re-initialization on the next search.
// =============================================================================

use std::sync::OnceLock;
use std::sync::RwLock as StdRwLock;

static MODEL: OnceLock<StdRwLock<Option<Arc<Mutex<EmbeddingModel>>>>> = OnceLock::new();

fn model_lock() -> &'static StdRwLock<Option<Arc<Mutex<EmbeddingModel>>>> {
    MODEL.get_or_init(|| StdRwLock::new(None))
}

/// Returns the shared embedding model, initializing it on the calling thread
/// the first time (or after a reset). Idempotent when already loaded.
///
/// Always called from `spawn_blocking` so the ~50 ms cold-start I/O never
/// blocks the async runtime.
pub fn get_or_init(app: &AppHandle) -> Result<Arc<Mutex<EmbeddingModel>>> {
    // Fast path: model already loaded.
    {
        let guard = model_lock().read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref m) = *guard {
            return Ok(m.clone());
        }
    }

    // Slow path: initialize under write lock.
    let mut guard = model_lock().write().unwrap_or_else(|e| e.into_inner());

    // Re-check after acquiring write lock (another thread may have raced us).
    if let Some(ref m) = *guard {
        return Ok(m.clone());
    }

    let model = Arc::new(Mutex::new(EmbeddingModel::init(app)?));
    *guard = Some(model.clone());
    Ok(model)
}

/// Drop the loaded model so the next call to `get_or_init` re-initializes it.
/// Called by the `reset_ai_state` Tauri command after an inference error.
pub fn reset_embedding_model() {
    let mut guard = model_lock().write().unwrap_or_else(|e| e.into_inner());
    *guard = None;
    tracing::info!("embedding model reset");
}

/// Whether the embedding model is initialized in-process.
pub fn is_loaded() -> bool {
    model_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

/// Whether the embedding model appears to be already downloaded to the on-disk
/// cache (so it will load without a network fetch). Best-effort: scans the
/// fastembed cache dir for a `.onnx` file. Distinct from [`is_loaded`], which
/// reports whether it's loaded into memory this session.
pub fn is_downloaded(app: &AppHandle) -> bool {
    let Ok(dir) = cache_dir(app) else {
        return false;
    };
    contains_onnx(&dir)
}

/// Recurse the cache dir looking for a `.onnx` file (fastembed nests the model
/// under a repo-named subdir). Returns on the first hit.
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

use std::sync::atomic::{AtomicBool, Ordering};
static WARMING: AtomicBool = AtomicBool::new(false);

/// Kick off a one-time background load (downloading the model on first ever use)
/// without blocking the caller. Safe to call repeatedly — the `WARMING` flag
/// dedups concurrent warms. Used by the orchestrator so a search never stalls
/// on a cold ~60 MB model download: that run falls back to lexical ranking and
/// semantic rerank kicks in on the next search once the model is ready.
pub fn warm_in_background(app: AppHandle) {
    if is_loaded() || WARMING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::task::spawn_blocking(move || {
        match get_or_init(&app) {
            Ok(_) => {
                tracing::info!("embedding model warmed and ready");
                // Notify the UI so the Settings row flips from
                // "loads on first search" to "ready".
                use tauri::Emitter;
                let _ = app.emit(
                    crate::events::EV_MODEL_STATUS,
                    crate::events::ModelStatusPayload {
                        model_id: "bge-small-en-v1.5".to_string(),
                        status: "embedding".to_string(),
                        detail: Some("ready".to_string()),
                    },
                );
            }
            Err(e) => tracing::warn!("embedding model warm failed: {e}"),
        }
        WARMING.store(false, Ordering::SeqCst);
    });
}

/// Semantic rerank step. Embeds the query and the (title + " " + abstract)
/// of every candidate, computes cosine similarity, normalizes to [0,1],
/// and blends with the existing Tier 1 score using
/// `final = 0.6*tier1 + 0.4*semantic`.
///
/// Runs entirely on a blocking thread because ort is sync. Caller awaits
/// via `tokio::task::spawn_blocking`.
///
/// `top_k` caps how many candidates we re-embed — keeping it bounded
/// guarantees latency is roughly linear in `top_k`, not in total candidates.
/// Items beyond `top_k` keep their Tier 1 score unchanged.
pub fn rerank_blocking(
    app: &AppHandle,
    query: &str,
    candidates: &mut [super::super::engine::ranking::RankedDoc],
    top_k: usize,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let model = get_or_init(app)?;
    // parking_lot::Mutex::lock() returns the guard directly — no Result, no poisoning.
    let mut model = model.lock();

    let query_emb = model
        .embed_batch(&[query.to_string()])?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("query embedding returned empty"))?;

    let limit = top_k.min(candidates.len());
    let texts: Vec<String> = candidates
        .iter()
        .take(limit)
        .map(|c| {
            let abstract_ = c.doc.doc.abstract_.as_deref().unwrap_or("");
            format!("{} {}", c.doc.doc.title, abstract_)
        })
        .collect();

    let doc_embs = model.embed_batch(&texts)?;
    drop(model);

    for (i, doc_emb) in doc_embs.iter().enumerate() {
        let sim = cosine(&query_emb, doc_emb).clamp(0.0, 1.0);
        candidates[i].score = 0.6 * candidates[i].score + 0.4 * sim;
    }

    candidates.sort_by(|a, b| {
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
    /// vector. Hits the network and pulls ~33 MB, so it's `#[ignore]`d by
    /// default — run explicitly with:
    /// `cargo test --no-default-features --features=custom-protocol,ai-embeddings -- --ignored`
    #[ignore]
    #[test]
    fn fastembed_downloads_and_embeds() {
        let tmp = std::env::temp_dir().join("df-fastembed-test");
        let _ = std::fs::create_dir_all(&tmp);
        let mut model = TextEmbedding::try_new(
            InitOptions::new(FastEmbedModel::BGESmallENV15Q)
                .with_cache_dir(tmp)
                .with_show_download_progress(true),
        )
        .expect("fastembed init/download failed");
        let out = model
            .embed(vec!["the quick brown fox"], None)
            .expect("embedding failed");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 384, "BGE-Small-EN-v1.5 is 384-dimensional");
    }
}
