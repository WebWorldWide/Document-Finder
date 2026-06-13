//! Shared mutable state for AI model availability + active downloads.
//!
//! Registered as Tauri-managed state via `app.manage(AiState::default())`.
//! All access goes through the typed methods so the lock is short-lived and
//! the cancel tokens stay synchronized with the status map.

use crate::ai::registry::{self, ModelEntry, ModelKind};
use crate::ai::storage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    NotDownloaded,
    Downloading { downloaded: u64, total: u64 },
    Verifying,
    Ready,
    Failed { msg: String },
    Cancelled,
}

#[derive(Default)]
pub struct AiState {
    /// model_id -> CancellationToken for an active download (if any).
    pub cancels: Mutex<HashMap<String, CancellationToken>>,
    /// model_id -> latest status. Updated by downloader event emissions
    /// (so the snapshot via `list` is correct without round-tripping
    /// through the frontend) AND on disk-state queries at startup.
    pub statuses: Mutex<HashMap<String, ModelStatus>>,
}

impl AiState {
    pub fn set_status(&self, model_id: &str, status: ModelStatus) {
        match self.statuses.lock() {
            Ok(mut s) => {
                s.insert(model_id.to_string(), status);
            }
            Err(e) => {
                tracing::warn!("AiState.set_status: statuses mutex poisoned: {}", e);
            }
        }
    }

    pub fn get_status(&self, model_id: &str) -> ModelStatus {
        self.statuses
            .lock()
            .ok()
            .and_then(|s| s.get(model_id).cloned())
            .unwrap_or(ModelStatus::NotDownloaded)
    }

    /// Atomically reserve the download slot for `model_id`: registers `token` and
    /// returns `true` only if no download was already in flight for that id.
    /// Combining the contains-check and the insert under a single lock acquisition
    /// closes a TOCTOU window where two concurrent `download_model` calls for the
    /// same id could both pass a separate `is_downloading` check and then race
    /// onto the same `.partial` file, corrupting it.
    pub fn try_begin_download(&self, model_id: &str, token: CancellationToken) -> bool {
        let mut c = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        if c.contains_key(model_id) {
            return false;
        }
        c.insert(model_id.to_string(), token);
        true
    }

    pub fn cancel_download(&self, model_id: &str) {
        // Recover from a poisoned lock (matches set_status) so a prior panic in a
        // cancel-map holder can't silently disable cancellation for the session.
        let mut c = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = c.remove(model_id) {
            t.cancel();
        }
    }

    pub fn clear_cancel(&self, model_id: &str) {
        let mut c = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        c.remove(model_id);
    }
}

/// One row returned by `list_models`. Combines static registry info with
/// live status + on-disk presence so the UI can render in one round trip.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: ModelKind,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
    pub license: &'static str,
    pub approx_bytes: u64,
    pub on_disk_bytes: u64,
    pub status: ModelStatus,
}

impl ModelInfo {
    pub fn from_entry(app: &AppHandle, entry: &ModelEntry, state: &AiState) -> Self {
        let on_disk = storage::model_file(app, entry)
            .ok()
            .map(|p| storage::on_disk_bytes(&p))
            .unwrap_or(0);

        // If the file is on disk but state hasn't been updated (e.g., fresh
        // app start), reflect that as Ready immediately.
        let mut status = state.get_status(entry.id);
        if matches!(status, ModelStatus::NotDownloaded) && on_disk > 0 {
            status = ModelStatus::Ready;
            state.set_status(entry.id, status.clone());
        }

        Self {
            id: entry.id.to_string(),
            kind: entry.kind,
            display_name: entry.display_name.to_string(),
            description: entry.description.to_string(),
            is_default: entry.is_default,
            license: entry.license,
            approx_bytes: entry.approx_bytes,
            on_disk_bytes: on_disk,
            status,
        }
    }
}

pub fn snapshot(app: &AppHandle, state: &AiState) -> Vec<ModelInfo> {
    registry::REGISTRY
        .iter()
        .map(|e| ModelInfo::from_entry(app, e, state))
        .collect()
}
