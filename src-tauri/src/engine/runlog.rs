//! Persistent JSONL run log.
//!
//! Every Discover run appends events here so failures, queries, and download
//! outcomes can be reviewed offline (and shared with whoever is debugging).
//!
//! Path:
//!   macOS  : ~/Library/Logs/Document Finder/runs.jsonl
//!   Linux  : ~/.local/state/document-finder/runs.jsonl
//!   Windows: %LOCALAPPDATA%\Document Finder\Logs\runs.jsonl

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::Lazy;

static LOG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub fn log_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        let dir = home.join("Library").join("Logs").join("Document Finder");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("runs.jsonl"))
    }
    #[cfg(target_os = "windows")]
    {
        let dir = dirs::data_local_dir()?.join("Document Finder").join("Logs");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("runs.jsonl"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let dir = dirs::state_dir()
            .or_else(dirs::data_local_dir)?
            .join("document-finder");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("runs.jsonl"))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event<'a> {
    RunStart {
        query: &'a str,
        sub_queries: &'a [String],
        sources: &'a [String],
    },
    SourceError {
        source: &'a str,
        error: &'a str,
        sub_query: Option<&'a str>,
    },
    Found {
        source: &'a str,
        title: &'a str,
        url: &'a str,
    },
    DownloadOk {
        source: &'a str,
        title: &'a str,
        url: &'a str,
        local_path: &'a str,
        bytes: u64,
    },
    DownloadFail {
        source: &'a str,
        title: &'a str,
        url: &'a str,
        error: &'a str,
    },
    RunComplete {
        done: usize,
        failed: usize,
        total: usize,
        folder: &'a str,
    },
}

pub fn log(event: Event<'_>) {
    let Some(path) = log_path() else { return };
    let payload = match serde_json::to_value(&event) {
        Ok(v) => v,
        Err(_) => return,
    };
    let line = serde_json::json!({
        "ts": Utc::now().to_rfc3339(),
        "event": payload,
    });
    let serialized = match serde_json::to_string(&line) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", serialized);
    }
}

/// Returns recent log lines as parsed JSON, newest first. Capped at `max`.
///
/// Reads only a bounded tail window of the file (scaled to `max`, clamped to a
/// few MB) rather than the whole log, so a long-lived `runs.jsonl` that has
/// grown to tens of megabytes can't pull it all into memory.
pub fn read_tail(max: usize) -> Vec<Value> {
    let Some(path) = log_path() else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return Vec::new();
    };

    // Budget ~2 KB per requested line, clamped to [64 KB, 4 MB], never past
    // the file length. Bounds memory regardless of `max` or file size.
    let window = (max as u64)
        .saturating_mul(2048)
        .clamp(64 * 1024, 4 * 1024 * 1024)
        .min(len);
    let start = len - window;
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }

    let mut bytes = Vec::with_capacity(window as usize);
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    // Lossy decode tolerates a multibyte char split at the window boundary;
    // that first (partial) line is dropped below when we seeked past the start.
    let buf = String::from_utf8_lossy(&bytes);

    let mut iter = buf.lines();
    if start > 0 {
        iter.next();
    }
    let mut lines: Vec<Value> = iter
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(max)
        .collect();
    lines.reverse();
    lines
}
