//! Local AI model management + inference for Tier 2 (semantic embeddings)
//! and Tier 3 (LLM query expansion + borderline filtering) of the search
//! pipeline.
//!
//! All models are downloaded from HuggingFace on first use, cached on disk,
//! and served from memory for the rest of the session. Nothing in this
//! module makes API calls to a paid service — the project goal is a fully
//! offline, fully free experience.

pub mod downloader;
pub mod registry;
pub mod state;
pub mod storage;

#[cfg(feature = "ai-embeddings")]
pub mod embed_worker;
#[cfg(feature = "ai-embeddings")]
pub mod embeddings;

#[cfg(feature = "ai-llm")]
pub mod llm;

pub use registry::{ModelEntry, ModelKind, REGISTRY};
pub use state::{AiState, ModelStatus};
