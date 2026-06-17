//! Streaming download with cancellation, content validation, landing-page→PDF
//! resolution, and a resume guard.

use futures::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::sources::Document;

/// Result of an individual download attempt.
pub enum DownloadOutcome {
    /// Newly downloaded to disk.
    Saved(PathBuf),
    /// An existing, valid file was reused (no bytes hit the network). Kept
    /// distinct from `Saved` so the UI never charges a cached file's size to
    /// the live network-throughput graph.
    Cached(PathBuf),
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
pub struct ProgressEvent {
    pub downloaded: u64,
    pub total: u64,
    /// When true, this is the terminal update for a file and must bypass the
    /// caller's time-based throttle so the final byte count is never dropped
    /// (the cause of a flat/under-counting throughput graph for fast or
    /// unknown-length downloads).
    pub force: bool,
}

/// Smallest size we'll accept for a "real" document. Sub-4 KB responses for
/// document URLs are almost always error pages.
const MIN_DOC_BYTES: u64 = 4 * 1024;

/// Hard cap per file — prevents a malicious or runaway server from exhausting disk.
const MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;

/// Cap on how much of an HTML landing page we'll read while hunting for a PDF
/// link. Real publisher pages put `citation_pdf_url` in the `<head>`, so this is
/// generous; it also bounds memory if a server streams a huge "HTML" body.
const HTML_RESOLVE_CAP: usize = 2 * 1024 * 1024;

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

/// Honest project contact for the Unpaywall API's `email` param (politeness, not
/// auth). A project address — never the user's — so no PII leaves the machine.
const UNPAYWALL_CONTACT: &str = "documentfinder@webworldwide.com";

/// Ask Unpaywall for an open-access PDF URL for a DOI, as a last-resort resolver
/// for a kept candidate whose primary link was paywalled / dead / a landing page
/// with no resolvable PDF. Returns `best_oa_location.url_for_pdf` when present.
/// Email param only (no key); the caller scopes this to DOI-bearing candidates.
pub async fn resolve_oa_pdf_via_unpaywall(client: &reqwest::Client, doi: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct UpResp {
        #[serde(default)]
        best_oa_location: Option<UpLoc>,
    }
    #[derive(serde::Deserialize)]
    struct UpLoc {
        #[serde(default)]
        url_for_pdf: Option<String>,
    }
    let url = format!("https://api.unpaywall.org/v2/{doi}");
    let resp = client
        .get(&url)
        .query(&[("email", UNPAYWALL_CONTACT)])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let body: UpResp = resp.json().await.ok()?;
    body.best_oa_location
        .and_then(|l| l.url_for_pdf)
        .filter(|u| !u.is_empty())
}

/// Cheap, pre-network URL rewrites that turn a known landing-page shape into a
/// direct document URL without a round-trip. Currently: arXiv `/abs/<id>` →
/// `/pdf/<id>` (the PDF endpoint serves `%PDF` directly).
pub fn canonicalize_doc_url(url: &str) -> String {
    if url.contains("arxiv.org/abs/") {
        return url.replace("arxiv.org/abs/", "arxiv.org/pdf/");
    }
    url.to_string()
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

// --- Landing-page → PDF resolution -----------------------------------------

static META_CITATION_PDF: Lazy<Regex> = Lazy::new(|| {
    // Require the token inside a name/property attribute — not anywhere in the
    // tag — so a stray mention in a description/content value can't false-match
    // and pre-empt the anchor fallback.
    Regex::new(r#"(?is)<meta\b[^>]*\b(?:name|property)\s*=\s*["']citation_pdf_url["'][^>]*>"#)
        .unwrap()
});
static LINK_PDF_ALTERNATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<link\b[^>]*\btype\s*=\s*["']application/pdf["'][^>]*>"#).unwrap()
});
static ATTR_CONTENT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)\bcontent\s*=\s*["']([^"']+)["']"#).unwrap());
static ATTR_HREF: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#).unwrap());
static ANCHOR_PDF: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*\bhref\s*=\s*["']([^"'#]+\.pdf(?:\?[^"']*)?)["']"#).unwrap()
});

/// Read at most `cap` bytes of a response body as lossy UTF-8. Used to scan an
/// HTML landing page without pulling an unbounded body into memory.
async fn read_body_capped(resp: reqwest::Response, cap: usize) -> Option<String> {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        buf.extend_from_slice(&chunk);
        if buf.len() >= cap {
            break;
        }
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Resolve a relative or absolute href against `base`, returning an absolute URL.
fn resolve_href(base: &url::Url, href: &str) -> Option<String> {
    base.join(href.trim()).ok().map(|u| u.to_string())
}

/// Hunt an HTML page for a direct PDF link. Priority, highest-yield first:
///   1. `<meta name="citation_pdf_url" content="…">` (the Google-Scholar tag
///      most scholarly publishers emit) — trusted, any host.
///   2. `<link rel="alternate" type="application/pdf" href="…">` — trusted.
///   3. The first **same-host** `<a href="….pdf">` — untrusted, so host-locked
///      to avoid grabbing an unrelated/advertised PDF.
///
/// Returns an absolute URL resolved against `base` (the page's final URL).
fn resolve_pdf_from_html(html: &str, base: &url::Url) -> Option<String> {
    if let Some(tag) = META_CITATION_PDF.find(html) {
        if let Some(c) = ATTR_CONTENT.captures(tag.as_str()) {
            if let Some(u) = resolve_href(base, &c[1]) {
                return Some(u);
            }
        }
    }
    if let Some(tag) = LINK_PDF_ALTERNATE.find(html) {
        if let Some(c) = ATTR_HREF.captures(tag.as_str()) {
            if let Some(u) = resolve_href(base, &c[1]) {
                return Some(u);
            }
        }
    }
    for cap in ANCHOR_PDF.captures_iter(html) {
        if let Some(abs) = resolve_href(base, &cap[1]) {
            if let Ok(u) = url::Url::parse(&abs) {
                if u.host_str() == base.host_str() {
                    return Some(abs);
                }
            }
        }
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
        // This runs only after the caller's `size >= MIN_DOC_BYTES` (4096) gate,
        // so a 4-byte read can't fail by being short — it's an I/O fault (e.g. a
        // Windows AV sharing-lock on the just-written file). Report it as such
        // rather than the misleading "too small", which matches the open branch.
        return Some("Couldn't verify the downloaded file.".to_string());
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

/// Positive document signature from a file's leading bytes. PDFs occasionally
/// carry a BOM / junk before `%PDF`, so we scan the first chunk rather than
/// requiring it at offset 0; an EPUB (ZIP) always starts with the local-file
/// header `PK\x03\x04`.
fn sniff_doc_ext(head: &[u8]) -> Option<&'static str> {
    if head.windows(4).any(|w| w == b"%PDF") {
        Some(".pdf")
    } else if head.starts_with(b"PK\x03\x04") {
        Some(".epub")
    } else {
        None
    }
}

/// Reconcile the extension guessed from the URL/Content-Type with the file's
/// real magic bytes. `ext_from` defaults an ambiguous type (e.g. a bare
/// `application/octet-stream` with no URL extension) to `.pdf`; this corrects
/// that so the file is stored — and later extracted — under its true type.
/// `claimed_binary` is true when the URL path or Content-Type explicitly
/// promised a PDF/EPUB. Returns the extension to store under, or `Err` when a
/// promised binary turned out not to be a document (an error page that slipped
/// past the Content-Type filter).
fn reconcile_doc_ext(
    head: &[u8],
    guessed_ext: &str,
    claimed_binary: bool,
) -> Result<&'static str, ()> {
    match sniff_doc_ext(head) {
        Some(ext) => Ok(ext),
        None if claimed_binary => Err(()),
        // Ambiguous type that isn't a known binary doc: keep it as text/markup
        // (a real .txt/.html document) rather than mislabeling it `.pdf`, which
        // would make the PDF extractor fail on perfectly good text.
        None => Ok(if guessed_ext == ".html" {
            ".html"
        } else {
            ".txt"
        }),
    }
}

/// Read up to the first `n` bytes of a file for magic-byte sniffing. Callers
/// guarantee the file is at least [`MIN_DOC_BYTES`] (4096), so a short read here
/// is an I/O fault, not EOF.
async fn read_file_head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; n];
    let read = file.read(&mut buf).await?;
    buf.truncate(read);
    Ok(buf)
}

/// GET a document URL with the document-shaped headers and bounded retry on
/// 429/5xx. Returns the validated `Response` or a terminal `DownloadOutcome`.
async fn get_document_response(
    client: &reqwest::Client,
    url: &str,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, DownloadOutcome> {
    let referer = referer_for(url);
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            return Err(DownloadOutcome::Cancelled);
        }
        let mut req = client
            .get(url)
            .header(reqwest::header::ACCEPT, DOC_ACCEPT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        if let Some(r) = &referer {
            req = req.header(reqwest::header::REFERER, r.clone());
        }
        match req.send().await {
            Ok(r) => match r.error_for_status() {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                    if attempt < 2 && (status == 429 || (500..=599).contains(&status)) {
                        let wait = std::time::Duration::from_secs(2 * (attempt + 1) as u64);
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    return Err(DownloadOutcome::Failed(format!(
                        "{}. (HTTP {status})",
                        http_status_message(status)
                    )));
                }
            },
            Err(_e) if attempt < 2 => {
                let wait = std::time::Duration::from_secs(2 * (attempt + 1) as u64);
                tokio::time::sleep(wait).await;
                continue;
            }
            Err(e) => return Err(DownloadOutcome::Failed(network_error_message(&e))),
        }
    }
    // The retry loop always returns inside the body; this guard just keeps a
    // future refactor from turning that into a panic.
    Err(DownloadOutcome::Failed(
        "Download failed after multiple attempts.".to_string(),
    ))
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
    let initial_url = canonicalize_doc_url(&doc.url);
    let initial_ext = ext_from(&initial_url, "");
    let mut out = dest_dir.join(format!("{}{}", doc.slug(), initial_ext));

    // Resume-skip: only honor an existing file if it passes magic-byte checks.
    // Older runs of this app saved HTML landing pages with .pdf extensions —
    // those need to be re-attempted (or rejected) rather than reused. A valid
    // existing file is reported as `Cached` (not `Saved`) and emits NO progress,
    // so its size is never charged to the live network-throughput graph.
    if let Ok(meta) = tokio::fs::metadata(&out).await {
        if meta.len() >= MIN_DOC_BYTES && existing_file_is_valid(&out).await {
            return DownloadOutcome::Cached(out);
        }
        // Stale junk — wipe so the new download has a clean slot.
        let _ = tokio::fs::remove_file(&out).await;
    }

    // Fetch loop: try the (canonicalized) document URL; if the server returns an
    // HTML landing page, resolve it to a real PDF link ONCE and retry. This is
    // the difference between "most web/meta-search results fail" and "they
    // download" — search engines overwhelmingly return landing-page URLs, not
    // direct PDFs.
    //
    // SSRF guard: every URL we connect to (the original AND any resolved hop)
    // is validated (http/https only, no credentials, resolves to a public IP)
    // before we connect. Redirects are re-checked by the client's redirect
    // policy (see `sources::make_client` / `make_download_client`).
    let mut target = initial_url;
    let mut resolved_once = false;
    let (resp, content_type) = loop {
        if let Err(reason) = crate::util::url_safety::validate_download_url(&target).await {
            return DownloadOutcome::Failed(format!("Blocked for safety: {reason}"));
        }
        let resp = match get_document_response(client, &target, cancel).await {
            Ok(r) => r,
            Err(outcome) => return outcome,
        };
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if reject_content_type(&target, &content_type).is_some() {
            // Landing page / JSON / XML. Try to resolve a PDF link from the
            // HTML body — but only once, to bound work and avoid loops.
            if !resolved_once {
                let base = resp.url().clone();
                let resolved = match read_body_capped(resp, HTML_RESOLVE_CAP).await {
                    Some(html) => resolve_pdf_from_html(&html, &base),
                    None => None,
                };
                if let Some(pdf_url) = resolved {
                    // Skip if the resolved link points back at the URL we just
                    // requested OR the page's own (post-redirect) final URL —
                    // re-fetching it would just return the same HTML.
                    if pdf_url != target && pdf_url != base.as_str() {
                        target = pdf_url;
                        resolved_once = true;
                        continue;
                    }
                }
            }
            return DownloadOutcome::Failed(
                "This link opened a web page, not a downloadable document.".to_string(),
            );
        }
        break (resp, content_type);
    };

    let final_ext = ext_from(&target, &content_type);
    if final_ext != initial_ext {
        out = dest_dir.join(format!("{}{}", doc.slug(), final_ext));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Pre-reject oversized files declared in Content-Length before writing
    // anything. (`total == 0` means "unknown length" and is never > the cap.)
    if total > MAX_FILE_BYTES {
        return DownloadOutcome::Failed(format!(
            "This file is too large to download (over {} MB).",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }

    // Stream into a sibling `.part` temp, then atomically rename to `out` only
    // after the bytes pass every validation. This guarantees the FINAL path never
    // holds a truncated body, so the resume-skip check above (which trusts a
    // 4-byte magic header) can never accept a half-written file left behind by a
    // crash / kill / power-loss mid-stream. One temp per doc-slug, in the same
    // dir, so the rename stays on one filesystem (atomic on all target OSes).
    let tmp = out.with_extension("part");
    let mut file = match File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => return DownloadOutcome::Failed(format!("Couldn't create the file on disk: {e}")),
    };

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    loop {
        let chunk_res = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp).await;
                return DownloadOutcome::Cancelled;
            }
        };
        let Some(chunk_res) = chunk_res else { break };
        let chunk = match chunk_res {
            Ok(c) => c,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp).await;
                return DownloadOutcome::Failed(network_error_message(&e));
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            return DownloadOutcome::Failed(format!("Couldn't save the file to disk: {e}"));
        }
        downloaded += chunk.len() as u64;
        if downloaded > MAX_FILE_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            return DownloadOutcome::Failed(format!(
                "This file is too large to download (over {} MB).",
                MAX_FILE_BYTES / 1024 / 1024
            ));
        }
        on_progress(ProgressEvent {
            downloaded,
            total,
            force: false,
        });
    }

    // Terminal, un-throttled progress event: guarantees the true final byte
    // count reaches the throughput graph even for fast or unknown-length
    // (chunked, total==0) downloads whose last chunk would otherwise be
    // suppressed by the caller's time-based throttle.
    on_progress(ProgressEvent {
        downloaded,
        total: if total == 0 { downloaded } else { total },
        force: true,
    });

    // fsync before rename so a power loss right after the rename can't surface a
    // metadata-but-no-data file; flush alone doesn't reach the disk.
    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return DownloadOutcome::Failed(format!("Couldn't finish saving the file to disk: {e}"));
    }
    if let Err(e) = file.sync_all().await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return DownloadOutcome::Failed(format!("Couldn't finish saving the file to disk: {e}"));
    }
    drop(file);

    // Validate the TEMP file before promoting it.
    let size = match tokio::fs::metadata(&tmp).await {
        Ok(m) => m.len(),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return DownloadOutcome::Failed(format!("Couldn't read the saved file: {e}"));
        }
    };
    if size == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return DownloadOutcome::Failed("The server returned no data.".to_string());
    }
    if size < MIN_DOC_BYTES {
        let _ = tokio::fs::remove_file(&tmp).await;
        return DownloadOutcome::Failed(format!(
            "The server returned only {size} bytes — likely an error page, not the document."
        ));
    }
    // Sniff the real magic bytes and reconcile the extension. (The temp file's
    // `.part` extension means the old `check_magic_bytes(&tmp)` was a no-op, so
    // the download path never validated bytes.) An ambiguous Content-Type that
    // `ext_from` defaulted to `.pdf` is re-typed to its true kind so extraction
    // works; a promised binary that isn't a document is rejected.
    let head = match read_file_head(&tmp, 1024).await {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return DownloadOutcome::Failed(format!("Couldn't verify the saved file: {e}"));
        }
    };
    let ct_lower = content_type.to_lowercase();
    let url_lower = target.to_lowercase();
    let url_path = url_lower.split('?').next().unwrap_or(&url_lower);
    let claimed_binary = ct_lower.contains("pdf")
        || ct_lower.contains("epub")
        || url_path.ends_with(".pdf")
        || url_path.ends_with(".epub");
    match reconcile_doc_ext(&head, final_ext, claimed_binary) {
        Ok(real_ext) => {
            if real_ext != final_ext {
                out = dest_dir.join(format!("{}{}", doc.slug(), real_ext));
            }
        }
        Err(()) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return DownloadOutcome::Failed(
                "The downloaded file wasn't a valid document (it was probably an error page)."
                    .to_string(),
            );
        }
    }

    // All checks passed — atomically publish the complete file.
    if let Err(e) = tokio::fs::rename(&tmp, &out).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return DownloadOutcome::Failed(format!("Couldn't finalize the download: {e}"));
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
    fn sniffs_pdf_and_epub_magic() {
        assert_eq!(sniff_doc_ext(b"%PDF-1.7\n..."), Some(".pdf"));
        assert_eq!(sniff_doc_ext(b"PK\x03\x04rest-of-zip"), Some(".epub"));
        // BOM / leading junk before %PDF is still detected.
        assert_eq!(sniff_doc_ext(b"\xef\xbb\xbf%PDF-1.4"), Some(".pdf"));
        assert_eq!(sniff_doc_ext(b"<!DOCTYPE html>"), None);
    }

    #[test]
    fn reconcile_reclassifies_ambiguous_binary() {
        // Defaulted to .pdf but the bytes are a ZIP/EPUB → re-typed to .epub.
        assert_eq!(
            reconcile_doc_ext(b"PK\x03\x04xx", ".pdf", false),
            Ok(".epub")
        );
        // Ambiguous (not claimed binary) and no signature → kept as text.
        assert_eq!(reconcile_doc_ext(b"plain text", ".pdf", false), Ok(".txt"));
        // A real PDF stays a PDF.
        assert_eq!(reconcile_doc_ext(b"%PDF-1.5", ".pdf", true), Ok(".pdf"));
        // Promised a PDF/EPUB but the bytes are neither → reject (error page).
        assert_eq!(reconcile_doc_ext(b"<html>nope", ".pdf", true), Err(()));
    }

    #[test]
    fn url_with_query_string_recognized_as_html() {
        assert!(url_claims_html("https://example.com/page.html?x=1"));
        assert!(!url_claims_html("https://example.com/paper.pdf?x=1"));
    }

    #[test]
    fn canonicalizes_arxiv_abs_to_pdf() {
        assert_eq!(
            canonicalize_doc_url("https://arxiv.org/abs/1706.03762"),
            "https://arxiv.org/pdf/1706.03762"
        );
        // Non-arxiv URLs pass through untouched.
        assert_eq!(
            canonicalize_doc_url("https://example.com/paper.pdf"),
            "https://example.com/paper.pdf"
        );
    }

    #[test]
    fn resolves_citation_pdf_url_meta() {
        let base = url::Url::parse("https://journal.org/articles/42").unwrap();
        let html = r#"<html><head>
            <meta name="citation_title" content="A Paper">
            <meta name="citation_pdf_url" content="https://journal.org/articles/42.pdf">
            </head><body>…</body></html>"#;
        assert_eq!(
            resolve_pdf_from_html(html, &base).as_deref(),
            Some("https://journal.org/articles/42.pdf")
        );
    }

    #[test]
    fn resolves_relative_pdf_link_against_base() {
        let base = url::Url::parse("https://repo.edu/record/9").unwrap();
        let html = r#"<a class="dl" href="/record/9/files/paper.pdf">Download PDF</a>"#;
        assert_eq!(
            resolve_pdf_from_html(html, &base).as_deref(),
            Some("https://repo.edu/record/9/files/paper.pdf")
        );
    }

    #[test]
    fn skips_cross_host_pdf_anchor() {
        let base = url::Url::parse("https://repo.edu/record/9").unwrap();
        // Only an off-host PDF anchor exists → not resolved (avoids ads/junk).
        let html = r#"<a href="https://ads.example.com/promo.pdf">ad</a>"#;
        assert_eq!(resolve_pdf_from_html(html, &base), None);
    }

    #[test]
    fn prefers_pdf_link_alternate_over_anchor() {
        let base = url::Url::parse("https://ojs.example.org/index.php/j/article/view/10").unwrap();
        let html = r#"<head><link rel="alternate" type="application/pdf" href="/index.php/j/article/view/10/9">
            </head><body><a href="/some/other.pdf">x</a></body>"#;
        assert_eq!(
            resolve_pdf_from_html(html, &base).as_deref(),
            Some("https://ojs.example.org/index.php/j/article/view/10/9")
        );
    }
}
