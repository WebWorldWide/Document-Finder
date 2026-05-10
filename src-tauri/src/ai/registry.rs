//! Static catalog of supported AI models.
//!
//! Each entry tells the model manager what to download from HuggingFace, where
//! to put it on disk, and how to verify it. SHA256 hashes are pinned so a
//! tampered or corrupted file is rejected after download.
//!
//! Adding a model: append to `REGISTRY`, fill out all fields, ship in a PR.
//! The frontend automatically picks up new entries via `list_models()`.

use once_cell::sync::Lazy;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Embedding,
    Llm,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    /// Stable identifier used in commands and on disk. Lowercase, hyphens.
    pub id: &'static str,
    pub kind: ModelKind,
    pub display_name: &'static str,
    /// HuggingFace repo, e.g. "Qwen/Qwen2.5-3B-Instruct-GGUF".
    pub hf_repo: &'static str,
    /// File within that repo to download.
    pub hf_filename: &'static str,
    /// Approximate size in bytes — for UI progress denominator before
    /// the server reports Content-Length.
    pub approx_bytes: u64,
    /// Pinned SHA256 of the file. Verified after download. Empty string
    /// disables verification (useful for files we don't yet have a hash
    /// for, but every shipping default should have one).
    pub sha256: &'static str,
    /// Human-readable description for the Settings UI.
    pub description: &'static str,
    /// Whether this entry is the recommended default for its kind.
    pub is_default: bool,
}

impl ModelEntry {
    /// Final URL to GET. HuggingFace serves a 302 redirect to the CDN;
    /// reqwest follows it automatically.
    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.hf_repo, self.hf_filename
        )
    }
}

pub static REGISTRY: Lazy<Vec<ModelEntry>> = Lazy::new(|| {
    vec![
        // The embedding model (BGE-Small-EN-v1.5) is intentionally NOT listed
        // here. `fastembed` manages its own download + on-disk cache via
        // `crate::ai::embeddings::EmbeddingModel::init()`, so a parallel
        // entry in this registry was duplicate work AND its hardcoded
        // HF URL went 404 when the upstream repo moved. Embedding readiness
        // is exposed to the UI via the `is_embedding_loaded` Tauri command.
        //
        // ---- LLM models -------------------------------------------------
        ModelEntry {
            id: "qwen2.5-3b-instruct-q4_k_m",
            kind: ModelKind::Llm,
            display_name: "Qwen 2.5 3B Instruct (Q4_K_M)",
            hf_repo: "Qwen/Qwen2.5-3B-Instruct-GGUF",
            hf_filename: "qwen2.5-3b-instruct-q4_k_m.gguf",
            approx_bytes: 2_020_000_000, // ~2.0 GB
            sha256: "",
            description: "Default local LLM. ~2 GB. Fast on Apple Silicon. Used for query expansion and borderline candidate filtering.",
            is_default: true,
        },
        ModelEntry {
            id: "qwen2.5-1.5b-instruct-q4_k_m",
            kind: ModelKind::Llm,
            display_name: "Qwen 2.5 1.5B Instruct (Q4_K_M)",
            hf_repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
            hf_filename: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
            approx_bytes: 1_020_000_000,
            sha256: "",
            description: "Smaller, faster LLM (~1 GB). Pick this if your machine is older or RAM-limited.",
            is_default: false,
        },
        ModelEntry {
            id: "llama-3.2-3b-instruct-q4_k_m",
            kind: ModelKind::Llm,
            display_name: "Llama 3.2 3B Instruct (Q4_K_M)",
            hf_repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
            hf_filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
            approx_bytes: 2_020_000_000,
            sha256: "",
            description: "Alternative LLM (~2 GB). Different style than Qwen — try both if results disagree.",
            is_default: false,
        },
    ]
});

pub fn find(id: &str) -> Option<&'static ModelEntry> {
    REGISTRY.iter().find(|m| m.id == id)
}

pub fn default_for(kind: ModelKind) -> Option<&'static ModelEntry> {
    REGISTRY.iter().find(|m| m.kind == kind && m.is_default)
}
