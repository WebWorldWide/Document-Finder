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

/// Accept header for document downloads. Biases content-negotiation toward real
/// documents but still accepts anything (`*/*`) so picky servers don't 406.
const DOC_ACCEPT: &str =
    "application/pdf,application/epub+zip,application/octet-stream;q=0.9,text/html;q=0.5,*/*;q=0.4";

/// A plain-language explanation for an HTTP error status, written for a
/// non-technical reader. The numeric code is appended separately (for logs).
fn http_status_message(status: u16) -> &'static str {
    match status {
        400 => {
            "The download link was rejected by the server — it may have expired or need a sign-in"
        }
        401 | 403 => "This source blocked the download — it may need a subscription or sign-in",
        404 | 410 => "The document is no longer available at this link",
        408 => "The server took too long to respond",
        429 => "The source is busy and asked us to slow down",
        500..=599 => "The source's server had a temporary problem",
        _ => "The server refused to send the file",
    }
}

/// Plain-language message for a transport-level (non-HTTP-status) error.
fn network_error_message(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "The download timed out — the server was too slow to respond.".to_string()
    } else if e.is_connect() {
        "Couldn't reach the server — check your internet connection.".to_string()
    } else {
        "Couldn't reach the server (network error).".to_string()
    }
}

/// Origin (`scheme://host/`) of a URL, used as a Referer. Many publisher servers
/// reject document requests that arrive without one. A tiny manual parse keeps
/// this dependency-free.
fn referer_for(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    let rest = &url[scheme_end + 3..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}/"))
    }
}

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
        return Some("This link opened a web page, not a downloadable document.".to_string());
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
        Err(_) => return Some("Couldn't verify the downloaded file.".to_string()),
    };
    let mut head = [0u8; 4];
    use tokio::io::AsyncReadExt;
    if file.read_exact(&mut head).await.is_err() {
        return Some("The downloaded file was too small to be a real document.".to_string());
    }

    match ext.as_str() {
        "pdf" => {
            if &head == b"%PDF" {
                None
            } else {
                Some(
                    "The downloaded file wasn't a valid PDF (it was probably an error page)."
                        .to_string(),
                )
            }
        }
        "epub" => {
            if &head == b"PK\x03\x04" {
                None
            } else {
                Some(
                    "The downloaded file wasn't a valid EPUB (it was probably an error page)."
                        .to_string(),
                )
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

    // SSRF guard: the document URL comes from arbitrary remote search results,
    // so validate it (http/https only, no credentials, resolves to a public IP)
    // before we connect. Redirects are re-checked by the client's redirect
    // policy (see `sources::make_client`).
    if let Err(reason) = crate::util::url_safety::validate_download_url(&doc.url).await {
        return DownloadOutcome::Failed(format!("Blocked for safety: {reason}"));
    }

    // Send document-shaped headers. A browser UA alone isn't enough for many
    // publisher CDNs: without an Accept that lists the document type and a
    // same-origin Referer, they answer 400/403. Adding both turns a chunk of
    // those rejections into successful downloads.
    let referer = referer_for(&doc.url);
    let mut resp = None;
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            return DownloadOutcome::Cancelled;
        }
        let mut req = client
            .get(&doc.url)
            .header(reqwest::header::ACCEPT, DOC_ACCEPT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        if let Some(r) = &referer {
            req = req.header(reqwest::header::REFERER, r.clone());
        }
        match req.send().await {
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
                    return DownloadOutcome::Failed(format!(
                        "{}. (HTTP {status})",
                        http_status_message(status)
                    ));
                }
            },
            Err(_e) if attempt < 2 => {
                let wait = std::time::Duration::from_secs(2 * (attempt + 1) as u64);
                tokio::time::sleep(wait).await;
                continue;
            }
            Err(e) => return DownloadOutcome::Failed(network_error_message(&e)),
        }
    }
    let Some(resp) = resp else {
        // The retry loop above always sets `resp` or returns early; this guard
        // just ensures a future refactor can't turn that into a panic.
        return DownloadOutcome::Failed("Download failed after multiple attempts.".to_string());
    };

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
        Err(e) => return DownloadOutcome::Failed(format!("Couldn't create the file on disk: {e}")),
    };

    // Pre-reject oversized files declared in Content-Length before writing
    // anything. (`total == 0` means "unknown length" and is never > the cap.)
    if total > MAX_FILE_BYTES {
        return DownloadOutcome::Failed(format!(
            "This file is too large to download (over {} MB).",
            MAX_FILE_BYTES / 1024 / 1024
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
                return DownloadOutcome::Failed(network_error_message(&e));
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&out).await;
            return DownloadOutcome::Failed(format!("Couldn't save the file to disk: {e}"));
        }
        downloaded += chunk.len() as u64;
        if downloaded > MAX_FILE_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&out).await;
            return DownloadOutcome::Failed(format!(
                "This file is too large to download (over {} MB).",
                MAX_FILE_BYTES / 1024 / 1024
            ));
        }
        on_progress(ProgressEvent { downloaded, total });
    }

    if let Err(e) = file.flush().await {
        return DownloadOutcome::Failed(format!("Couldn't finish saving the file to disk: {e}"));
    }
    drop(file);

    let size = match tokio::fs::metadata(&out).await {
        Ok(m) => m.len(),
        Err(e) => return DownloadOutcome::Failed(format!("Couldn't read the saved file: {e}")),
    };
    if size == 0 {
        let _ = tokio::fs::remove_file(&out).await;
        return DownloadOutcome::Failed("The server returned no data.".to_string());
    }
    if size < MIN_DOC_BYTES {
        let _ = tokio::fs::remove_file(&out).await;
        return DownloadOutcome::Failed(format!(
            "The server returned only {size} bytes — likely an error page, not the document."
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
