//! Semantic embeddings via `fastembed`.
//!
//! Uses BGE-Small-EN-v1.5 (384-dim, ~33 MB quantized ONNX). On first call,
//! `fastembed` downloads the model + tokenizer to its own cache directory
//! and builds an inference session backed by ONNX Runtime. Subsequent calls
//! reuse the loaded session.
//!
//! Inference is synchronous (ort is blocking); we wrap calls in
//! `tokio::task::spawn_blocking` so the orchestrator's async runtime
//! stays responsive while reranking runs.
//!
//! The model is cheap enough that even on CPU-only machines a 100-doc
//! rerank takes ~1-2 seconds. With CoreML/CUDA it drops to ~100ms.

use anyhow::Context;
use fastembed::{EmbeddingModel as FastEmbedModel, InitOptions, TextEmbedding};
use std::sync::{Arc, Mutex, OnceLock};

pub struct EmbeddingModel {
    inner: TextEmbedding,
}

impl EmbeddingModel {
    /// Initialize the model. First call downloads + caches; later calls are
    /// effectively free.
    pub fn init() -> anyhow::Result<Self> {
        let inner = TextEmbedding::try_new(
            InitOptions::new(FastEmbedModel::BGESmallENV15)
                .with_show_download_progress(true),
        )
        .context("fastembed init failed")?;
        Ok(Self { inner })
    }

    /// Embed a batch of texts. Returns one Vec<f32> per input. Internal
    /// batching is handled by fastembed (default size 256, plenty).
    /// Requires `&mut self` because the underlying ONNX session is stateful.
    pub fn embed_batch(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        // fastembed wants Vec<&str>; cheap to construct.
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
// Singleton — load once per process. The Mutex serializes inference calls
// (the underlying ONNX session is single-threaded per instance, so parallel
// embed calls would have to copy session state anyway).
// =============================================================================

static MODEL: OnceLock<Arc<Mutex<EmbeddingModel>>> = OnceLock::new();

/// Returns the shared embedding model, initializing it on the calling
/// thread the first time. Idempotent — subsequent calls are O(1).
///
/// Costs ~1s of model download + load on cold start. Always called from
/// `spawn_blocking` so it can never block the async runtime.
pub fn get_or_init() -> anyhow::Result<Arc<Mutex<EmbeddingModel>>> {
    if let Some(m) = MODEL.get() {
        return Ok(m.clone());
    }
    let model = Arc::new(Mutex::new(EmbeddingModel::init()?));
    let _ = MODEL.set(model.clone());
    Ok(model)
}

/// Whether the embedding model is initialized in-process. Useful for
/// "ready" status reporting without triggering a load.
pub fn is_loaded() -> bool {
    MODEL.get().is_some()
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
    query: &str,
    candidates: &mut [super::super::engine::ranking::RankedDoc],
    top_k: usize,
) -> anyhow::Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let model = get_or_init()?;
    let mut model = model.lock().map_err(|_| anyhow::anyhow!("embedding mutex poisoned"))?;

    // Embed the query alone first.
    let query_emb = model.embed_batch(&[query.to_string()])?.pop()
        .ok_or_else(|| anyhow::anyhow!("query embedding returned empty"))?;

    let limit = top_k.min(candidates.len());
    // Build the input texts only for the top_k candidates.
    let texts: Vec<String> = candidates
        .iter()
        .take(limit)
        .map(|c| {
            let abstract_ = c.doc.doc.abstract_.as_deref().unwrap_or("");
            format!("{} {}", c.doc.doc.title, abstract_)
        })
        .collect();

    let doc_embs = model.embed_batch(&texts)?;
    drop(model); // Release lock before scoring.

    // Compute cosine + blend.
    for (i, doc_emb) in doc_embs.iter().enumerate() {
        let sim = cosine(&query_emb, doc_emb).clamp(0.0, 1.0);
        // Map cosine [-1, 1] (clamped above to [0, 1]) into a multiplier.
        // Keep the original Tier 1 score as-is, then layer semantic on top.
        // Equivalent of `final = 0.6 * tier1_norm + 0.4 * semantic` after
        // recombining — we mutate `score` in place.
        candidates[i].score = 0.6 * candidates[i].score + 0.4 * sim;
    }

    // Re-sort.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(())
}
