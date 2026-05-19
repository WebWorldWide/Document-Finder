//! Semantic embeddings via `fastembed`.
//!
//! BGE-Small-EN-v1.5 (384-dim, ~33 MB quantized ONNX) ships **bundled** in
//! the app — no network download at runtime. The five model files live under
//! `src-tauri/resources/embeddings/bge-small-en-v1.5/` and are registered as
//! Tauri bundle resources, so `app.path().resource_dir()` resolves to a
//! readable directory containing them in both dev and packaged builds.
//!
//! On first call we read the bytes into memory and hand them to
//! `TextEmbedding::try_new_from_user_defined` — this sidesteps fastembed's
//! built-in HuggingFace download path entirely (whose hardcoded URL had
//! gone 404 upstream, the cause of the previous "embeddings broken" bug).
//!
//! Inference is synchronous (ort is blocking); we wrap calls in
//! `tokio::task::spawn_blocking` so the orchestrator's async runtime
//! stays responsive while reranking runs.

use anyhow::{Context, Result};
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub struct EmbeddingModel {
    inner: TextEmbedding,
}

/// Subdirectory under the bundle's resource dir where the BGE files live.
const BGE_RESOURCE_SUBDIR: &str = "resources/embeddings/bge-small-en-v1.5";

fn resolve_bge_dir(app: &AppHandle) -> Result<PathBuf> {
    let resource_dir = app
        .path()
        .resource_dir()
        .context("could not resolve Tauri resource directory")?;
    let bge = resource_dir.join(BGE_RESOURCE_SUBDIR);
    if !bge.exists() {
        anyhow::bail!(
            "bundled BGE embedding model not found at {} — did `bundle.resources` get out of sync?",
            bge.display()
        );
    }
    Ok(bge)
}

impl EmbeddingModel {
    /// Build the model from the bundled resource files. Reads ~35 MB of
    /// bytes off disk synchronously — fast on any reasonable machine and
    /// only happens once per process via the singleton below.
    pub fn init(app: &AppHandle) -> Result<Self> {
        let dir = resolve_bge_dir(app)?;

        let onnx = std::fs::read(dir.join("model_quantized.onnx"))
            .context("reading bundled model_quantized.onnx")?;
        let tokenizer_file =
            std::fs::read(dir.join("tokenizer.json")).context("reading bundled tokenizer.json")?;
        let config_file =
            std::fs::read(dir.join("config.json")).context("reading bundled config.json")?;
        let tokenizer_config_file = std::fs::read(dir.join("tokenizer_config.json"))
            .context("reading bundled tokenizer_config.json")?;
        let special_tokens_map_file = std::fs::read(dir.join("special_tokens_map.json"))
            .context("reading bundled special_tokens_map.json")?;

        let tokenizer_files = TokenizerFiles {
            tokenizer_file,
            config_file,
            special_tokens_map_file,
            tokenizer_config_file,
        };

        let user_model = UserDefinedEmbeddingModel::new(onnx, tokenizer_files)
            .with_pooling(Pooling::Cls)
            .with_quantization(QuantizationMode::Static);

        let inner =
            TextEmbedding::try_new_from_user_defined(user_model, InitOptionsUserDefined::new())
                .context("fastembed user-defined init failed (bundled BGE files)")?;

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
