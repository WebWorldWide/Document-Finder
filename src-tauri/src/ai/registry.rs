//! Static catalog of supported AI models.
//!
//! Each entry tells the model manager what to download from HuggingFace, where
//! to put it on disk, and how to verify it. SHA256 hashes are pinned so a
//! tampered or corrupted file is rejected after download.
//!
//! Licensing: this app is AGPL-3.0-or-later, so the catalog ships only models
//! under permissive licenses (Apache-2.0) that any user — including commercial
//! — may run without extra restrictions. Models with non-commercial or
//! custom-restricted terms (e.g. Qwen's "qwen-research", Meta's Llama Community
//! License, Mistral's Research License) are intentionally excluded as defaults.
//!
//! Adding a model: append to `REGISTRY`, fill out all fields (including a
//! permissive `license`), ship in a PR. The frontend automatically picks up new
//! entries via `list_models()`.

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
    /// Pinned SHA256 of the file, verified after download so a tampered or
    /// corrupted file is rejected and re-fetched (taken from the file's
    /// HuggingFace LFS ETag). An empty string would disable verification, so the
    /// `every_entry_pins_a_valid_sha256` test enforces that every shipping entry
    /// pins a real 64-hex-char hash — you can't accidentally ship an unpinned
    /// (unverified) model.
    pub sha256: &'static str,
    /// SPDX license identifier of the model weights (e.g. "Apache-2.0").
    /// The catalog ships only permissive licenses compatible with this app's
    /// AGPL-3.0-or-later license; surfaced in the Settings UI so users see the
    /// terms before downloading.
    pub license: &'static str,
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
        // Only Apache-2.0 models — small, permissive, and strong at the two
        // lightweight tasks we use an LLM for (query expansion + borderline
        // candidate filtering). Bigger models aren't needed for this work.
        ModelEntry {
            id: "qwen2.5-1.5b-instruct-q4_k_m",
            kind: ModelKind::Llm,
            display_name: "Qwen 2.5 1.5B Instruct (Q4_K_M)",
            hf_repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
            hf_filename: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
            approx_bytes: 1_117_320_736, // ~1.1 GB
            sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
            license: "Apache-2.0",
            description: "Default local LLM (~1.1 GB). Best-in-class at its size and fully permissive. Used for query expansion and borderline candidate filtering.",
            is_default: true,
        },
        ModelEntry {
            id: "qwen2.5-0.5b-instruct-q4_k_m",
            kind: ModelKind::Llm,
            display_name: "Qwen 2.5 0.5B Instruct (Q4_K_M)",
            hf_repo: "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            hf_filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
            approx_bytes: 491_400_032, // ~470 MB
            sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
            license: "Apache-2.0",
            description: "Ultra-light LLM (~470 MB). Pick this on older or RAM-limited machines; slightly lower quality than the 1.5B.",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every catalog entry must pin a real SHA256. An empty (or malformed) pin
    /// silently disables post-download verification in `downloader::download`,
    /// so this guards against ever shipping an unverified model.
    #[test]
    fn every_entry_pins_a_valid_sha256() {
        for entry in REGISTRY.iter() {
            assert_eq!(
                entry.sha256.len(),
                64,
                "model '{}' must pin a 64-hex-char SHA256 (got {} chars); an empty/short pin disables download verification",
                entry.id,
                entry.sha256.len()
            );
            assert!(
                entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "model '{}' SHA256 must be lowercase hex",
                entry.id
            );
        }
    }
}
