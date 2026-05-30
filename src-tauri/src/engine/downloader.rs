//! Streaming download with cancellation, content validation, and resume guard.

use futures::StreamExt;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::sources::Document;

/// Result of an individual download attempt.
pub enum DownloadOutcome {
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
pub struct ProgressEvent {
    pub downloaded: u64,
    pub total: u64,
}

/// Smallest size we'll accept for a "real" document. Sub-4 KB responses for
/// document URLs are almost always error pages.
const MIN_DOC_BYTES: u64 = 4 * 1024;

/// Hard cap per file — prevents a malicious or runaway server from exhausting disk.
const MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;

pub fn ext_from(url: &str, content_type: &str) -> &'static str {
    let ct = content_type.to_lowercase();
    let u = url.to_lowercase();
    let path_only = u.split('?').next().unwrap_or(&u);

    if ct.contains("pdf") || path_only.ends_with(".pdf") {
        ".pdf"
    } else if ct.contains("epub") || path_only.ends_with(".epub") {
        ".epub"
    } else if ct.contains("html") || path_only.ends_with(".html") || path_only.ends_with(".htm") {
        ".html"
    } else if ct.contains("text") || ct.contains("plain") || path_only.ends_with(".txt") {
        ".txt"
    } else {
        ".pdf"
    }
}

fn url_claims_html(url: &str) -> bool {
    let u = url.to_lowercase();
    let path = u.split('?').next().unwrap_or(&u);
    path.ends_with(".html") || path.ends_with(".htm")
}

/// Returns Some(reason) if the Content-Type indicates we got a non-document
/// response (HTML/JSON/XML landing page) when the URL didn't claim to be HTML.
fn reject_content_type(url: &str, content_type: &str) -> Option<String> {
    let ct = content_type.to_lowercase();
    if url_claims_html(url) {
        return None;
    }
    if ct.contains("text/html")
        || ct.contains("application/xhtml")
        || ct.contains("application/json")
        || ct.contains("application/xml")
        || ct.contains("text/xml")
    {
        return Some(format!(
            "Skipped: Landing page (detected {})",
            content_type
                .split(';')
                .next()
                .unwrap_or(content_type)
                .trim()
        ));
    }
    None
}

/// Verify the file on disk has the expected magic bytes for its extension.
/// Returns None if valid, Some(reason) otherwise.
async fn check_magic_bytes(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !matches!(ext.as_str(), "pdf" | "epub") {
        return None;
    }

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => return Some("could not open downloaded file for validation".to_string()),
    };
    let mut head = [0u8; 4];
    use tokio::io::AsyncReadExt;
    if file.read_exact(&mut head).await.is_err() {
        return Some("file too short to validate magic bytes".to_string());
    }

    match ext.as_str() {
        "pdf" => {
            if &head == b"%PDF" {
                None
            } else {
                Some("downloaded file is not a valid PDF (invalid signature)".to_string())
            }
        }
        "epub" => {
            if &head == b"PK\x03\x04" {
                None
            } else {
                Some("downloaded file is not a valid EPUB (invalid zip signature)".to_string())
            }
        }
        _ => None,
    }
}

/// True if the existing file on disk passes magic-byte validation for its
/// extension. Used by the resume-skip path so we don't keep junk from a
/// previous run with a less strict downloader.
async fn existing_file_is_valid(path: &Path) -> bool {
    check_magic_bytes(path).await.is_none()
}

pub async fn download<F>(
    doc: &Document,
    dest_dir: &Path,
    client: &reqwest::Client,
    cancel: &CancellationToken,
    mut on_progress: F,
) -> DownloadOutcome
where
    F: FnMut(ProgressEvent) + Send,
{
    let initial_ext = ext_from(&doc.url, "");
    let mut out = dest_dir.join(format!("{}{}", doc.slug(), initial_ext));

    // Resume-skip: only honor an existing file if it passes magic-byte checks.
    // Older runs of this app saved HTML landing pages with .pdf extensions —
    // those need to be re-attempted (or rejected) rather than reused.
    if let Ok(meta) = tokio::fs::metadata(&out).await {
        if meta.len() >= MIN_DOC_BYTES && existing_file_is_valid(&out).await {
            on_progress(ProgressEvent {
                downloaded: meta.len(),
                total: meta.len(),
            });
            return DownloadOutcome::Saved(out);
        }
        // Stale junk — wipe so the new download has a clean slot.
        let _ = tokio::fs::remove_file(&out).await;
    }

    let mut resp = None;
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            return DownloadOutcome::Cancelled;
        }
        match client.get(&doc.url).send().await {
            Ok(r) => match r.error_for_status() {
                Ok(r) => {
                    resp = Some(r);
                    break;
                }
                Err(e) => {
                    let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                    if attempt < 2 && (status == 429 || (500..=599).contains(&status)) {
                        let wait = std::time::Duration::from_secs(2 * (attempt + 1) as u64);
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    return DownloadOutcome::Failed(format!("HTTP {}: {}", status, e));
                }
            },
            Err(_e) if attempt < 2 => {
                let wait = std::time::Duration::from_secs(2 * (attempt + 1) as u64);
                tokio::time::sleep(wait).await;
                continue;
            }
            Err(e) => return DownloadOutcome::Failed(format!("Network error: {}", e)),
        }
    }
    let resp = resp.unwrap();

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Early reject: the URL claimed to be a document but the server is serving
    // an HTML landing page / JSON / XML wrapper. Don't waste bandwidth.
    if let Some(reason) = reject_content_type(&doc.url, &content_type) {
        return DownloadOutcome::Failed(reason);
    }

    let final_ext = ext_from(&doc.url, &content_type);
    if final_ext != initial_ext {
        out = dest_dir.join(format!("{}{}", doc.slug(), final_ext));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut file = match File::create(&out).await {
        Ok(f) => f,
        Err(e) => return DownloadOutcome::Failed(e.to_string()),
    };

    // Pre-reject oversized files declared in Content-Length before writing
    // anything. (`total == 0` means "unknown length" and is never > the cap.)
    if total > MAX_FILE_BYTES {
        return DownloadOutcome::Failed(format!(
            "file too large ({} bytes declared in Content-Length)",
            total
        ));
    }

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    loop {
        let chunk_res = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => {
                drop(file);
                let _ = tokio::fs::remove_file(&out).await;
                return DownloadOutcome::Cancelled;
            }
        };
        let Some(chunk_res) = chunk_res else { break };
        let chunk = match chunk_res {
            Ok(c) => c,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&out).await;
                return DownloadOutcome::Failed(e.to_string());
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&out).await;
            return DownloadOutcome::Failed(e.to_string());
        }
        downloaded += chunk.len() as u64;
        if downloaded > MAX_FILE_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&out).await;
            return DownloadOutcome::Failed(format!(
                "file too large (exceeded {} MB limit)",
                MAX_FILE_BYTES / 1024 / 1024
            ));
        }
        on_progress(ProgressEvent { downloaded, total });
    }

    if let Err(e) = file.flush().await {
        return DownloadOutcome::Failed(e.to_string());
    }
    drop(file);

    let size = match tokio::fs::metadata(&out).await {
        Ok(m) => m.len(),
        Err(e) => return DownloadOutcome::Failed(e.to_string()),
    };
    if size == 0 {
        let _ = tokio::fs::remove_file(&out).await;
        return DownloadOutcome::Failed("empty response".to_string());
    }
    if size < MIN_DOC_BYTES {
        let _ = tokio::fs::remove_file(&out).await;
        return DownloadOutcome::Failed(format!(
            "response too small ({size} bytes) — likely an error page"
        ));
    }

    if let Some(reason) = check_magic_bytes(&out).await {
        let _ = tokio::fs::remove_file(&out).await;
        return DownloadOutcome::Failed(reason);
    }

    DownloadOutcome::Saved(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_html_for_pdf_url() {
        let r = reject_content_type("https://example.com/paper.pdf", "text/html; charset=utf-8");
        assert!(r.is_some(), "should reject HTML at .pdf URL");
    }

    #[test]
    fn allows_html_for_html_url() {
        let r = reject_content_type("https://example.com/page.html", "text/html");
        assert!(r.is_none(), "html at .html url is fine");
    }

    #[test]
    fn rejects_json_for_pdf_url() {
        let r = reject_content_type("https://example.com/paper.pdf", "application/json");
        assert!(r.is_some());
    }

    #[test]
    fn allows_pdf_content_type() {
        let r = reject_content_type("https://example.com/paper.pdf", "application/pdf");
        assert!(r.is_none());
    }

    #[test]
    fn url_with_query_string_recognized_as_html() {
        assert!(url_claims_html("https://example.com/page.html?x=1"));
        assert!(!url_claims_html("https://example.com/paper.pdf?x=1"));
    }
}
