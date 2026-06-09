//! Local LLM inference via `llama-cpp-2` (bindings to llama.cpp).
//!
//! Loads a GGUF model from `{app_data}/models/{model_id}/{filename}` —
//! downloaded by the E1 model manager — and exposes two task-shaped methods:
//!
//!   * `generate` — free-form completion with a token cap.
//!   * `yes_no` — bounded 1-2 token classification used by Tier 3's borderline-candidate filter.
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
use std::sync::{Arc, OnceLock, RwLock};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

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
        // Default to CPU-only. The GPU/Metal backend can NATIVE-ABORT (SIGABRT)
        // during load OR during new_context/decode — NOT a catchable Rust `Err`,
        // so the panic hook and the `Err -> CPU fallback` below can't see it, and
        // it takes the whole app down with no recovery. This is especially likely
        // in a quarantined, ad-hoc-signed macOS .dmg where Metal shader runtime
        // compilation can fail. The CPU backend avoids that abort surface; the
        // models this app runs are tiny (0.5–1.5B Q4, used for query expansion +
        // a yes/no filter, and expansion runs concurrently with discovery), so
        // CPU is fast enough. Power users can opt back into GPU with `DF_GPU=1`.
        // `DF_CPU_ONLY` / `GGML_NO_ACCEL` are still honored (force CPU) for
        // back-compat and override the opt-in.
        let force_cpu =
            std::env::var("DF_CPU_ONLY").is_ok() || std::env::var("GGML_NO_ACCEL").is_ok();
        let try_gpu = !force_cpu && std::env::var("DF_GPU").is_ok();

        if !try_gpu {
            tracing::info!("Loading GGUF model on CPU (set DF_GPU=1 to opt into GPU/Metal).");
            let params = LlamaModelParams::default().with_n_gpu_layers(0);
            let model = LlamaModel::load_from_file(backend, path, &params)
                .context("LlamaModel::load_from_file failed (CPU)")?;
            return Ok(Self { model });
        }

        tracing::info!("DF_GPU set — attempting GPU-accelerated LLM load...");
        let params = LlamaModelParams::default().with_n_gpu_layers(999);
        match LlamaModel::load_from_file(backend, path, &params) {
            Ok(model) => Ok(Self { model }),
            Err(e) => {
                tracing::warn!(
                    "GPU LLM load failed: {}. Falling back to CPU... (a native GPU abort can't be caught here)",
                    e
                );
                let cpu_params = LlamaModelParams::default().with_n_gpu_layers(0);
                let model = LlamaModel::load_from_file(backend, path, &cpu_params)
                    .context("LlamaModel::load_from_file failed on CPU fallback")?;
                Ok(Self { model })
            }
        }
    }

    /// Generate up to `max_tokens` from `prompt`. Stops early on EOS, or when
    /// `cancel` fires. The cancel check matters because this runs on a blocking
    /// thread holding the model mutex: without it, a Stop (or a timed-out
    /// `select!` in the caller) detaches the task but the decode keeps running to
    /// the full token budget, pinning the singleton mutex and a blocking-pool
    /// thread until it finishes. Checking per token bounds that to ~one decode.
    pub fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        cancel: &CancellationToken,
    ) -> anyhow::Result<String> {
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

        // `cur_pos` is the absolute KV-cache position (prompt length + tokens
        // generated so far), fed to `batch.add`; the loop index isn't a drop-in
        // substitute, so the explicit counter is intentional.
        #[allow(clippy::explicit_counter_loop)]
        for _ in 0..max_tokens {
            // Cooperative cancellation: a detached/timed-out caller can't abort
            // this blocking thread, so bail here to release the model mutex.
            if cancel.is_cancelled() {
                break;
            }
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
    pub fn yes_no(&self, prompt: &str, cancel: &CancellationToken) -> anyhow::Result<bool> {
        let raw = self.generate(prompt, 6, cancel)?;
        Ok(parse_yes(&raw))
    }
}

fn parse_yes(raw: &str) -> bool {
    let first = raw
        .trim()
        .split(|c: char| !c.is_alphabetic())
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(first.as_str(), "y" | "yes" | "true")
}

// =============================================================================
// Singleton — one model per process, resettable on demand.
//
// Uses RwLock<Option<...>> so reset_llm_model() can drop the model and force
// re-initialization on the next search. Inference is gated by the inner
// AsyncMutex because llama.cpp contexts can't run two decodes in parallel.
// =============================================================================

static MODEL: OnceLock<RwLock<Option<Arc<AsyncMutex<LlmModel>>>>> = OnceLock::new();

fn model_lock() -> &'static RwLock<Option<Arc<AsyncMutex<LlmModel>>>> {
    MODEL.get_or_init(|| RwLock::new(None))
}

/// Load the LLM if not already loaded. Idempotent — second call returns the
/// cached handle without touching disk. Always called from `spawn_blocking`.
pub fn load_blocking(path: &Path) -> anyhow::Result<Arc<AsyncMutex<LlmModel>>> {
    // Fast path.
    {
        let guard = model_lock().read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref m) = *guard {
            return Ok(m.clone());
        }
    }

    // Slow path: initialize under write lock.
    let mut guard = model_lock().write().unwrap_or_else(|e| e.into_inner());
    if let Some(ref m) = *guard {
        return Ok(m.clone());
    }

    let model = Arc::new(AsyncMutex::new(LlmModel::load(path)?));
    *guard = Some(model.clone());
    Ok(model)
}

pub fn try_get() -> Option<Arc<AsyncMutex<LlmModel>>> {
    model_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Drop the loaded model so the next call to `load_blocking` re-initializes.
/// Called by the `reset_ai_state` Tauri command after an inference error.
pub fn reset_llm_model() {
    let mut guard = model_lock().write().unwrap_or_else(|e| e.into_inner());
    *guard = None;
    tracing::info!("llm model reset");
}

pub fn is_loaded() -> bool {
    model_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

// =============================================================================
// Prompt templates — kept here so the orchestrator stays narrative.
// =============================================================================

pub fn expansion_prompt(query: &str) -> String {
    // Few-shot, raw-completion prompt (the model is fed un-templated — see
    // `generate`). The worked example teaches the breadth we want: take ONE
    // topic and fan it out into many search phrases that each hit a different
    // facet (causes, effects, treatment, demographics, prevention, statistics,
    // history, comparisons, …). The "Topic:/Queries:" structure lets
    // `parse_expansion` cleanly cut the model off if it rolls into a second
    // hallucinated example. We want WIDE reach, so we ask for many lines.
    format!(
        "You are an expert research librarian. Given a topic, you generate a broad set of search \
        queries that approach it from many different angles, so a search engine surfaces the \
        widest possible range of relevant documents. First silently fix any spelling mistakes, \
        then output the corrected topic itself followed by many varied queries — one per line, no \
        numbering, no quotes, no commentary. Cover whichever of these angles fit the topic: causes \
        and risk factors; effects and consequences; treatments, solutions, and how-to; specific \
        groups (children, teenagers, adults, by country); prevention; statistics and prevalence; \
        history and background; definitions and overviews; symptoms or warning signs; comparisons \
        with related topics; controversies and debates; and academic or technical framings. Do not \
        copy the example below verbatim — it only shows the style.\n\
        \n\
        Topic: gaming addiction\n\
        Queries:\n\
        gaming addiction\n\
        what causes gaming addiction\n\
        how to overcome gaming addiction\n\
        gaming addiction in adults\n\
        gaming addiction in teenagers and children\n\
        gaming addiction symptoms and warning signs\n\
        video game addiction treatment options\n\
        gaming addiction statistics and prevalence\n\
        effects of gaming addiction on mental health\n\
        preventing video game addiction\n\
        internet gaming disorder diagnosis criteria\n\
        gaming addiction versus gambling addiction\n\
        online gaming addiction and social isolation\n\
        psychology of compulsive video gaming\n\
        \n\
        Topic: {q}\n\
        Queries:\n",
        q = query
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

/// How many LLM-generated sub-queries we keep, at most. Kept high on purpose:
/// the product's value is broad reach, and discovery is throttled per-source
/// (see `orchestrator::discover_wave`) so a wide fan-out doesn't rate-limit any
/// single source.
pub const MAX_EXPANSION_SUBQUERIES: usize = 24;

/// Parse the LLM's expansion output into a clean Vec of unique sub-queries.
///
/// Drops numbering/bullets/quotes, blank lines, and duplicates of the original
/// query. The expansion prompt is few-shot with `Topic:`/`Queries:` headers, so
/// if the model rolls past our topic into a fresh (hallucinated) example we stop
/// at its `Topic:` line rather than ingesting that example's queries.
pub fn parse_expansion(raw: &str, original: &str) -> Vec<String> {
    let original_lc = original.trim().to_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(original_lc.clone());

    for line in raw.lines() {
        let line = line.trim();
        let lower = line.to_lowercase();
        // A new few-shot example block begins — everything after belongs to a
        // different topic, so cut the model off here.
        if lower.starts_with("topic:") {
            break;
        }
        // Skip the echoed section header and blank lines (don't stop on blanks:
        // some models put a stray blank inside the list).
        if line.is_empty() || lower.starts_with("queries:") {
            continue;
        }
        let trimmed = line
            .trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*' || c == '•'
            })
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if trimmed.is_empty() || trimmed.len() < 3 || trimmed.len() > 200 {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            out.push(trimmed);
        }
        if out.len() >= MAX_EXPANSION_SUBQUERIES {
            break;
        }
    }
    out
}

/// Prompt for a one-line spelling correction of a search query.
pub fn spellfix_prompt(query: &str) -> String {
    format!(
        "Correct any spelling mistakes or typos in this search query. Keep the meaning and all \
        correctly-spelled words unchanged — only fix misspellings. Reply with ONLY the corrected \
        query on a single line, nothing else.\n\
        \n\
        Query: {}\n\
        Corrected:",
        query
    )
}

/// Extract a spelling-corrected query from the LLM reply. Returns `Some` only
/// for a sane, single-line correction that actually differs from the original
/// and isn't suspiciously long (a hallucination or echoed prompt/prose).
pub fn parse_spellfix(raw: &str, original: &str) -> Option<String> {
    let line = raw
        .lines()
        .map(|l| l.trim().trim_matches('"').trim_matches('\'').trim())
        .find(|l| !l.is_empty())?;
    if line.is_empty() || line.len() > original.trim().len() + 24 {
        return None;
    }
    if line.eq_ignore_ascii_case(original.trim()) {
        return None; // no change
    }
    let lc = line.to_lowercase();
    if [
        "query",
        "corrected",
        "sorry",
        "cannot",
        "i ",
        "the corrected",
    ]
    .iter()
    .any(|bad| lc.contains(bad))
    {
        return None; // echoed prompt or prose, not a bare query
    }
    Some(line.to_string())
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
    fn parse_expansion_stops_at_next_topic_header() {
        // The few-shot model rolled into a second, unrelated example. We must
        // keep only our topic's queries and drop the hallucinated continuation.
        let raw = "gaming addiction\n\
                   what causes gaming addiction\n\
                   gaming addiction in adults\n\
                   \n\
                   Topic: climate change\n\
                   Queries:\n\
                   causes of climate change\n";
        let out = parse_expansion(raw, "gaming addiction");
        assert!(out.contains(&"what causes gaming addiction".to_string()));
        assert!(out.contains(&"gaming addiction in adults".to_string()));
        assert!(
            !out.iter().any(|s| s.to_lowercase().contains("climate")),
            "leaked a query from the next example: {out:?}"
        );
        // The echoed original is dropped; "Queries:" header is skipped.
        assert!(!out
            .iter()
            .any(|s| s.eq_ignore_ascii_case("gaming addiction")));
        assert!(!out.iter().any(|s| s.to_lowercase().starts_with("queries")));
    }

    #[test]
    fn parse_expansion_caps_output() {
        let mut raw = String::new();
        for i in 0..60 {
            raw.push_str(&format!("unique sub query number {i}\n"));
        }
        let out = parse_expansion(&raw, "orig");
        assert_eq!(out.len(), MAX_EXPANSION_SUBQUERIES);
    }

    #[test]
    fn filter_prompt_truncates_long_abstract() {
        let abs = "x".repeat(2000);
        let p = filter_prompt("q", "t", &abs);
        // 800 chars of abstract + the surrounding chrome.
        assert!(p.len() < 1500);
    }
}
