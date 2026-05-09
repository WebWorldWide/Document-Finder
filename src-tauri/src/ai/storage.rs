//! On-disk layout for downloaded AI models.
//!
//! Lives under Tauri's per-app data directory:
//!     {app_data}/document-finder/models/{model_id}/{filename}
//!
//! The per-model subdirectory keeps the model file plus any sidecar files
//! it needs (tokenizer.json, config.json) together. For E1's pinned set the
//! main file is the only artifact, but we keep the directory layout so
//! future entries that need sidecars don't force a refactor.

use crate::ai::registry::ModelEntry;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Returns the root models directory, creating it if needed.
pub fn models_root(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow::anyhow!("could not resolve app_data_dir: {}", e))?;
    let root = base.join("models");
    std::fs::create_dir_all(&root)
        .map_err(|e| anyhow::anyhow!("create models dir {}: {}", root.display(), e))?;
    Ok(root)
}

pub fn model_dir(app: &AppHandle, entry: &ModelEntry) -> anyhow::Result<PathBuf> {
    let dir = models_root(app)?.join(entry.id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create model dir {}: {}", dir.display(), e))?;
    Ok(dir)
}

pub fn model_file(app: &AppHandle, entry: &ModelEntry) -> anyhow::Result<PathBuf> {
    Ok(model_dir(app, entry)?.join(entry.hf_filename))
}

pub fn partial_file(app: &AppHandle, entry: &ModelEntry) -> anyhow::Result<PathBuf> {
    Ok(model_dir(app, entry)?.join(format!("{}.partial", entry.hf_filename)))
}

/// File size in bytes if the model is fully downloaded, else 0.
pub fn on_disk_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
