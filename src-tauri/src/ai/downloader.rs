//! Streaming model download with resume + SHA256 verification.
//!
//! Downloads land at `{model_dir}/{filename}.partial`, get verified, then
//! get atomically renamed to `{filename}`. If the user cancels mid-download
//! or the process crashes, the next attempt resumes from the partial via
//! HTTP Range requests when the server supports them.
//!
//! Distinct from `engine/downloader.rs` (which downloads research PDFs):
//! - much larger files (~2 GB vs ~5 MB)
//! - resume mandatory (a 2 GB restart from byte 0 is unacceptable)
//! - SHA256 verification on completion
//! - emits separate event types for the model UI

use crate::ai::registry::ModelEntry;
use crate::ai::storage::{model_file, partial_file};
use crate::events::{ModelProgressPayload, ModelStatusPayload, EV_MODEL_PROGRESS, EV_MODEL_STATUS};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

fn emit_status(app: &AppHandle, model_id: &str, status: &str, detail: Option<String>) {
    let _ = app.emit(
        EV_MODEL_STATUS,
        ModelStatusPayload {
            model_id: model_id.to_string(),
            status: status.to_string(),
            detail,
        },
    );
}

fn emit_progress(app: &AppHandle, model_id: &str, downloaded: u64, total: u64, bytes_per_sec: u64) {
    let _ = app.emit(
        EV_MODEL_PROGRESS,
        ModelProgressPayload {
            model_id: model_id.to_string(),
            downloaded,
            total,
            bytes_per_sec,
        },
    );
}

/// Run the download to completion (or until cancelled). Returns the final
/// model file path on success.
pub async fn download(
    app: AppHandle,
    client: Arc<reqwest::Client>,
    entry: &'static ModelEntry,
    cancel: CancellationToken,
) -> anyhow::Result<PathBuf> {
    let final_path = model_file(&app, entry)?;
    let partial_path = partial_file(&app, entry)?;

    // Already-complete short-circuit: if the final file exists and either
    // the SHA matches or no SHA is pinned, we're done.
    if final_path.exists() {
        if entry.sha256.is_empty() || verify_sha256(&final_path, entry.sha256).await? {
            emit_status(&app, entry.id, "ready", Some("already downloaded".into()));
            return Ok(final_path);
        }
        // Pinned hash didn't match — file is corrupt. Remove and re-download.
        let _ = tokio::fs::remove_file(&final_path).await;
    }

    // Disk-space precheck before we commit to writing 2 GB. Subtract anything
    // already on disk in the partial so resume of a 90%-done file doesn't
    // false-positive on a near-full disk.
    let already_partial = tokio::fs::metadata(&partial_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let still_needed = entry.approx_bytes.saturating_sub(already_partial);
    if let Some(parent) = partial_path.parent() {
        if let Err(msg) = crate::ai::storage::ensure_free_space(parent, still_needed) {
            emit_status(&app, entry.id, "failed", Some(msg.clone()));
            return Err(anyhow::anyhow!(msg));
        }
    }

    emit_status(&app, entry.id, "downloading", None);

    // Resume offset: where the partial file leaves off, if any.
    let mut start = already_partial;

    let url = entry.download_url();
    let mut req = client.get(&url);
    if start > 0 {
        req = req.header("Range", format!("bytes={}-", start));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            // Distinguish connect/DNS errors from generic transport errors so
            // the user sees something they can act on.
            let detail = if e.is_connect() {
                format!("could not connect to huggingface.co: {}", e)
            } else if e.is_timeout() {
                format!("connection timed out: {}", e)
            } else {
                format!("network error: {}", e)
            };
            emit_status(&app, entry.id, "failed", Some(detail.clone()));
            return Err(anyhow::anyhow!(detail));
        }
    };

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        let code = status.as_u16();
        let detail = match code {
            401 => format!(
                "HuggingFace returned 401 Unauthorized for {}/{} — this model may require accepting a license. Visit https://huggingface.co/{} in a browser, accept terms, then retry.",
                entry.hf_repo, entry.hf_filename, entry.hf_repo
            ),
            403 => format!(
                "HuggingFace returned 403 Forbidden for {}/{} — the file may be gated or blocked in your region. Try a different model.",
                entry.hf_repo, entry.hf_filename
            ),
            404 => format!(
                "HuggingFace returned 404 — model file moved or removed: {}/{}. Update Document-Finder for a fresh registry.",
                entry.hf_repo, entry.hf_filename
            ),
            429 => "HuggingFace rate-limited the download (429). Wait a minute and retry.".to_string(),
            451 => "Download blocked for legal reasons (451) in your region.".to_string(),
            500..=599 => format!("HuggingFace server error {} — retry in a moment.", code),
            _ => format!("HTTP {} from {}", code, url),
        };
        emit_status(&app, entry.id, "failed", Some(detail.clone()));
        return Err(anyhow::anyhow!(detail));
    }

    // Compute total bytes from Content-Length (fallback: registry estimate).
    // For 206 partial responses, Content-Length is the *remaining* size.
    let remaining = resp
        .content_length()
        .unwrap_or_else(|| entry.approx_bytes.saturating_sub(start));
    let total = if status.as_u16() == 206 {
        start + remaining
    } else {
        // Server ignored our Range header; throw away our partial.
        start = 0;
        remaining
    };

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(&partial_path)
        .await?;

    let mut downloaded = start;
    let mut last_emit = std::time::Instant::now();
    let mut last_emit_bytes = downloaded;
    let mut stream = resp.bytes_stream();

    while let Some(chunk_res) = stream.next().await {
        if cancel.is_cancelled() {
            file.flush().await?;
            emit_status(&app, entry.id, "cancelled", None);
            return Err(anyhow::anyhow!("cancelled"));
        }
        let chunk = match chunk_res {
            Ok(c) => c,
            Err(e) => {
                let detail = format!(
                    "transfer interrupted at {} of {} bytes: {}",
                    downloaded, total, e
                );
                emit_status(&app, entry.id, "failed", Some(detail.clone()));
                return Err(anyhow::anyhow!(detail));
            }
        };
        if let Err(e) = file.write_all(&chunk).await {
            let detail = format!("disk write failed: {} (free space may have run out)", e);
            emit_status(&app, entry.id, "failed", Some(detail.clone()));
            return Err(anyhow::anyhow!(detail));
        }
        downloaded += chunk.len() as u64;

        // Throttle progress events to ~10/sec.
        if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
            let elapsed = last_emit.elapsed().as_secs_f64().max(0.001);
            let bps = ((downloaded - last_emit_bytes) as f64 / elapsed) as u64;
            emit_progress(&app, entry.id, downloaded, total, bps);
            last_emit = std::time::Instant::now();
            last_emit_bytes = downloaded;
        }
    }
    file.flush().await?;
    drop(file);
    emit_progress(&app, entry.id, downloaded, total, 0);

    // Verify SHA if pinned.
    if !entry.sha256.is_empty() {
        emit_status(&app, entry.id, "verifying", None);
        if !verify_sha256(&partial_path, entry.sha256).await? {
            let _ = tokio::fs::remove_file(&partial_path).await;
            emit_status(&app, entry.id, "failed", Some("sha256 mismatch".into()));
            return Err(anyhow::anyhow!("sha256 mismatch for {}", entry.id));
        }
    }

    // Atomic rename → final path.
    tokio::fs::rename(&partial_path, &final_path).await?;
    emit_status(&app, entry.id, "ready", None);
    Ok(final_path)
}

/// Stream the file through SHA256. Done on a blocking thread because file I/O
/// at this size hits the disk hard and we don't want to monopolize the
/// async runtime.
async fn verify_sha256(path: &PathBuf, expected: &str) -> anyhow::Result<bool> {
    let path = path.clone();
    let expected = expected.to_lowercase();
    tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual = hex::encode(hasher.finalize());
        Ok(actual == expected)
    })
    .await?
}
