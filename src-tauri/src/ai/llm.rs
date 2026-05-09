//! Local LLM inference via `llama-cpp-2` (bindings to llama.cpp).
//!
//! Loads a GGUF model from `{app_data}/models/{model_id}/{filename}` —
//! downloaded by the E1 model manager — and exposes two task-shaped methods:
//!
//!   * `generate` — free-form completion with a token cap.
//!   * `yes_no`   — bounded 1-2 token classification used by Tier 3's
//!                  borderline-candidate filter.
//!
//! Inference is single-threaded per context (llama.cpp limitation) so the
//! singleton wraps the model in `tokio::sync::Mutex`. Loading runs on a
//! blocking thread because `LlamaModel::load_from_file` does heavy disk I/O
//! plus mmap; we never want to stall the async runtime on it.

use anyhow::Context;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// One backend per process — initializing twice is undefined behavior.
fn backend() -> anyhow::Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    if let Some(b) = BACKEND.get() {
        return Ok(b);
    }
    let b = LlamaBackend::init().context("LlamaBackend::init failed")?;
    let _ = BACKEND.set(b);
    Ok(BACKEND.get().expect("just set"))
}

pub struct LlmModel {
    model: LlamaModel,
}

impl LlmModel {
    /// Load a GGUF model from disk. Heavy: triggers mmap + metadata parse.
    /// Always called from `spawn_blocking`.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let backend = backend()?;
        let mut params = LlamaModelParams::default();
        // Use all available GPU layers — llama.cpp picks Metal on macOS,
        // CUDA on NVIDIA, etc., automatically when the binary was compiled
        // with the relevant backend.
        params = params.with_n_gpu_layers(999);
        let model = LlamaModel::load_from_file(backend, path, &params)
            .context("LlamaModel::load_from_file failed")?;
        Ok(Self { model })
    }

    /// Generate up to `max_tokens` from `prompt`. Stops early on EOS.
    pub fn generate(&self, prompt: &str, max_tokens: usize) -> anyhow::Result<String> {
        let backend = backend()?;
        let mut ctx_params = LlamaContextParams::default();
        ctx_params = ctx_params.with_n_ctx(std::num::NonZeroU32::new(2048));
        let mut ctx = self
            .model
            .new_context(backend, ctx_params)
            .context("new_context failed")?;

        let prompt_tokens = self
            .model
            .str_to_token(prompt, AddBos::Always)
            .context("tokenize failed")?;
        if prompt_tokens.is_empty() {
            return Ok(String::new());
        }

        // Feed the entire prompt in one batch.
        let mut batch = LlamaBatch::new(prompt_tokens.len().max(512), 1);
        let n_prompt = prompt_tokens.len() as i32;
        for (i, tok) in prompt_tokens.iter().enumerate() {
            // Only ask for logits on the very last prompt token — that's
            // where sampling starts.
            let is_last = i == prompt_tokens.len() - 1;
            batch
                .add(*tok, i as i32, &[0], is_last)
                .context("batch.add prompt token")?;
        }
        ctx.decode(&mut batch).context("initial decode failed")?;

        // Sampler chain: low-randomness, top-p constrained. For our use case
        // (query expansion + yes/no filter) deterministic-ish output is fine.
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.4),
            LlamaSampler::dist(1234),
        ]);

        let mut output = String::new();
        let mut cur_pos = n_prompt;
        let mut last_logits_idx = (prompt_tokens.len() - 1) as i32;
        // Persistent UTF-8 decoder so multi-byte chars split across tokens
        // assemble correctly.
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        for _ in 0..max_tokens {
            let next = sampler.sample(&ctx, last_logits_idx);
            sampler.accept(next);

            if self.model.is_eog_token(next) {
                break;
            }

            let piece = self
                .model
                .token_to_piece(next, &mut decoder, false, None)
                .unwrap_or_default();
            output.push_str(&piece);

            // Decode the new token to extend the context for the next sample.
            batch.clear();
            batch
                .add(next, cur_pos, &[0], true)
                .context("batch.add gen token")?;
            ctx.decode(&mut batch).context("gen decode failed")?;
            cur_pos += 1;
            last_logits_idx = 0; // After clear+add, index 0 is the only token.
        }

        Ok(output)
    }

    /// Yes/no classification — generates up to 4 tokens and parses the
    /// leading word as a boolean. "y", "yes" → true; "n", "no" → false;
    /// anything else also → false (conservative).
    pub fn yes_no(&self, prompt: &str) -> anyhow::Result<bool> {
        let raw = self.generate(prompt, 6)?;
        let first = raw
            .trim()
            .split(|c: char| !c.is_alphabetic())
            .next()
            .unwrap_or("")
            .to_lowercase();
        Ok(matches!(first.as_str(), "y" | "yes" | "true"))
    }
}

// =============================================================================
// Singleton — one model per process. Inference is gated by an async mutex
// because llama.cpp contexts can't run two decodes in parallel.
// =============================================================================

static MODEL: OnceLock<Arc<AsyncMutex<LlmModel>>> = OnceLock::new();

/// Load the LLM if not already loaded. Idempotent — second call returns
/// the cached handle without touching disk.
pub fn load_blocking(path: &Path) -> anyhow::Result<Arc<AsyncMutex<LlmModel>>> {
    if let Some(m) = MODEL.get() {
        return Ok(m.clone());
    }
    let model = Arc::new(AsyncMutex::new(LlmModel::load(path)?));
    let _ = MODEL.set(model.clone());
    Ok(model)
}

pub fn try_get() -> Option<Arc<AsyncMutex<LlmModel>>> {
    MODEL.get().cloned()
}

pub fn is_loaded() -> bool {
    MODEL.get().is_some()
}

// =============================================================================
// Prompt templates — kept here so the orchestrator stays narrative.
// =============================================================================

pub fn expansion_prompt(query: &str) -> String {
    format!(
        "You are an expert research librarian. The user is searching for documents about: \"{}\"\n\
        \n\
        Generate 5 alternative search queries that would surface documents about this topic from \
        different angles. Include domain synonyms, formal/informal terms, and historical or \
        contemporary phrasings. Output each query on its own line. No numbering, no preamble, \
        no quotes.\n\
        \n\
        Alternative queries:\n",
        query
    )
}

pub fn filter_prompt(query: &str, title: &str, abstract_: &str) -> String {
    let abst = if abstract_.is_empty() {
        "(no abstract)"
    } else {
        abstract_
    };
    format!(
        "Is the following document relevant to the search query \"{q}\"? Answer only \"yes\" or \"no\".\n\
        \n\
        Title: {t}\n\
        Abstract: {a}\n\
        \n\
        Answer:",
        q = query,
        t = title,
        a = &abst.chars().take(800).collect::<String>(),
    )
}

/// Parse the LLM's expansion output into a clean Vec of unique sub-queries.
/// Drops empty lines, common preambles, and duplicates of the original query.
pub fn parse_expansion(raw: &str, original: &str) -> Vec<String> {
    let original_lc = original.trim().to_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(original_lc.clone());

    for line in raw.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == '-')
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if trimmed.is_empty() || trimmed.len() < 3 || trimmed.len() > 200 {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            out.push(trimmed);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_expansion_dedup_and_strips_numbering() {
        let raw = "1. American Civil War 1861-1865\n\
                   2) antebellum political documents\n\
                   - civil war primary sources\n\
                   civil war\n\
                   \n\
                   Reconstruction era papers";
        let out = parse_expansion(raw, "civil war");
        assert!(out.contains(&"American Civil War 1861-1865".to_string()));
        assert!(out.contains(&"antebellum political documents".to_string()));
        assert!(out.contains(&"Reconstruction era papers".to_string()));
        assert!(!out.iter().any(|s| s.to_lowercase() == "civil war"));
    }

    #[test]
    fn filter_prompt_truncates_long_abstract() {
        let abs = "x".repeat(2000);
        let p = filter_prompt("q", "t", &abs);
        // 800 chars of abstract + the surrounding chrome.
        assert!(p.len() < 1500);
    }
}
