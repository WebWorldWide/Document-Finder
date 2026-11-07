//! Streaming download with cancellation, content validation, landing-page→PDF
//! resolution, and a resume guard.

use futures::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::hostlimit::HostLimiter;
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
/// Research papers are almost always under 20 MB; anything this large is a
/// scanned book or a dataset that would stall a run for little text.
const MAX_FILE_BYTES: u64 = 120 * 1024 * 1024;

/// Hard wall-clock cap on one file's body transfer.
const MAX_TRANSFER: std::time::Duration = std::time::Duration::from_secs(75);
/// After this long, a transfer must be making real progress.
const SLOW_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
/// Below this average rate (after `SLOW_GRACE`) the file is abandoned.
const MIN_RATE_BPS: f64 = 48.0 * 1024.0;
/// Abandon a known-size file whose projected finish is further off than this.
const MAX_PROJECTED: std::time::Duration = std::time::Duration::from_secs(60);

/// Failure message for a transfer abandoned by the speed caps.
pub const SLOW_MSG: &str = "The server was too slow, so we skipped this copy.";

/// Cap on how much of an HTML landing page we'll read while hunting for a PDF
/// link. Real publisher pages put `citation_pdf_url` in the `<head>`, so this is
/// generous; it also bounds memory if a server streams a huge "HTML" body.
const HTML_RESOLVE_CAP: usize = 2 * 1024 * 1024;

/// Accept header for document downloads. Biases content-negotiation toward real
/// documents but still accepts anything (`*/*`) so picky servers don't 406.
const DOC_ACCEPT: &str =
    "application/pdf,application/epub+zip,application/octet-stream;q=0.9,text/html;q=0.5,*/*;q=0.4";

/// Honest tool User-Agent for hosts whose WAF blocks the browser UA. The shared
/// download client sends a browser UA (publisher PDF hosts like Sage/OUP/Brill
/// 403 generic crawlers), but some OPEN repositories do the opposite and 403 the
/// browser UA as a suspected scraper — Zenodo is the confirmed case.
const REPO_UA: &str = "DocumentFinder/0.1 (open-access research tool)";

/// True if `url`'s host is a repository known to reject the browser User-Agent
/// (and serve normally for an honest one). Checked per-request in the downloader.
fn host_needs_honest_ua(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("://zenodo.org/") || u.contains(".zenodo.org/")
}

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

/// Ask Unpaywall for open-access PDF URLs for a DOI (best location first,
/// then up to two others). Email param only (no key).
pub async fn resolve_oa_pdfs_via_unpaywall(client: &reqwest::Client, doi: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct UpResp {
        #[serde(default)]
        best_oa_location: Option<UpLoc>,
        #[serde(default)]
        oa_locations: Vec<UpLoc>,
    }
    #[derive(serde::Deserialize)]
    struct UpLoc {
        #[serde(default)]
        url_for_pdf: Option<String>,
    }
    let url = format!("https://api.unpaywall.org/v2/{doi}");
    let fetch = async {
        let resp = client
            .get(&url)
            .query(&[("email", UNPAYWALL_CONTACT)])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        resp.json::<UpResp>().await.ok()
    };
    let Some(body) = tokio::time::timeout(std::time::Duration::from_secs(8), fetch)
        .await
        .ok()
        .flatten()
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for loc in body.best_oa_location.into_iter().chain(body.oa_locations) {
        if let Some(u) = loc.url_for_pdf.filter(|u| !u.is_empty()) {
            if !out.contains(&u) {
                out.push(u);
            }
        }
        if out.len() >= 3 {
            break;
        }
    }
    out
}

static PMC_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(PMC\d{4,})\b").unwrap());
static OSF_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^https?://osf\.io/([a-z0-9]{5})/?$").unwrap());
static ARXIV_DOI: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)doi\.org/10\.48550/arxiv\.(\d{4}\.\d{4,5}(?:v\d+)?)").unwrap());

/// Cheap, pre-network rewrites that turn known landing-page shapes into a
/// direct document URL, saving a round-trip (and often a failure):
/// arXiv abs/DOI → PDF, PMC / Europe PMC articles → Europe PMC's PDF render,
/// bioRxiv / medRxiv → `.full.pdf`, OSF project → `/download`.
pub fn canonicalize_doc_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.contains("arxiv.org/abs/") {
        return url.replace("arxiv.org/abs/", "arxiv.org/pdf/");
    }
    if let Some(c) = ARXIV_DOI.captures(url) {
        return format!("https://arxiv.org/pdf/{}", &c[1]);
    }
    let pmc_page = (lower.contains("ncbi.nlm.nih.gov/pmc/articles/")
        || lower.contains("pmc.ncbi.nlm.nih.gov/articles/")
        || lower.contains("europepmc.org/article/pmc/")
        || lower.contains("europepmc.org/articles/"))
        && !lower.contains(".pdf");
    if pmc_page {
        if let Some(c) = PMC_ID.captures(url) {
            return format!(
                "https://europepmc.org/backend/ptpmcrender.fcgi?accid={}&blobtype=pdf",
                c[1].to_uppercase()
            );
        }
    }
    if (lower.contains("biorxiv.org/content/10.") || lower.contains("medrxiv.org/content/10."))
        && !lower.ends_with(".pdf")
        && !lower.contains('?')
    {
        let base = url
            .trim_end_matches('/')
            .trim_end_matches(".full")
            .trim_end_matches(".abstract");
        return format!("{base}.full.pdf");
    }
    if let Some(c) = OSF_ID.captures(&lower) {
        return format!("https://osf.io/{}/download", &c[1]);
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
// JS-driven download widgets stash the real PDF in a data-* attribute
// (`data-pdf-url`, `data-pdf`, `data-href`) instead of a plain href.
static DATA_PDF_ATTR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)\bdata-(?:pdf-url|pdf|href)\s*=\s*["']([^"'#]+?\.pdf(?:\?[^"']*)?)["']"#)
        .unwrap()
});
// Repository / journal "Download" endpoints that serve a PDF from a path with no
// `.pdf` extension (e.g. `/download/123`, `/article/view/10/9/bitstream`). Tried
// only as a last resort and host-locked, so a wrong guess just fails the
// content-type check rather than fetching an unrelated file.
static ANCHOR_DOWNLOADISH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?is)<a\b[^>]*\bhref\s*=\s*["']([^"'#]*/(?:download|fulltext|bitstream|getpdf|viewfile|pdfviewer|pdf)(?:/[^"'#]*)?)["']"#,
    )
    .unwrap()
});

/// Resolve a relative or absolute href against `base`, returning an absolute URL.
fn resolve_href(base: &url::Url, href: &str) -> Option<String> {
    base.join(href.trim()).ok().map(|u| u.to_string())
}

/// Hunt an HTML page for a direct PDF link. Priority, highest-yield first:
///   1. `<meta name="citation_pdf_url" content="…">` (the Google-Scholar tag
///      most scholarly publishers emit) — trusted, any host.
///   2. `<link rel="alternate" type="application/pdf" href="…">` — trusted.
///   3. A `data-pdf-url` / `data-pdf` / `data-href="….pdf"` attribute (JS
///      download widgets) — host-locked.
///   4. The first **same-host** `<a href="….pdf">` — host-locked.
///   5. Last resort: a **same-host** download-style anchor (`/download/…`,
///      `/bitstream/…`, `/pdf/…`, …) with no `.pdf` extension. Host-locked; a
///      wrong guess just fails the downloader's content-type/magic check.
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
    // data-* PDF attributes (host-locked, like the anchor fallbacks).
    if let Some(u) = first_same_host_match(&DATA_PDF_ATTR, html, base) {
        return Some(u);
    }
    if let Some(u) = first_same_host_match(&ANCHOR_PDF, html, base) {
        return Some(u);
    }
    // Last resort: a download-style endpoint without a `.pdf` extension.
    first_same_host_match(&ANCHOR_DOWNLOADISH, html, base)
}

/// Return the first capture-group-1 match of `re` in `html` that resolves to an
/// absolute URL on the same host as `base`. Shared by the host-locked
/// (untrusted) PDF-link fallbacks so they can't grab an off-host / advertised file.
fn first_same_host_match(re: &Regex, html: &str, base: &url::Url) -> Option<String> {
    for cap in re.captures_iter(html) {
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

    // Read a small head and scan it the same way the main download path does
    // (`sniff_doc_ext`): a real PDF can carry a BOM / leading junk before `%PDF`,
    // so requiring the signature at offset 0 (the old check) wrongly rejected
    // valid cached PDFs and forced a needless re-download every run. Callers
    // guarantee the file is >= MIN_DOC_BYTES, so a short read here is an I/O
    // fault, not EOF.
    let head = match read_file_head(path, 1024).await {
        Ok(h) => h,
        Err(_) => return Some("Couldn't verify the downloaded file.".to_string()),
    };

    match ext.as_str() {
        "pdf" => {
            if head.windows(4).any(|w| w == b"%PDF") {
                None
            } else {
                Some(
                    "The downloaded file wasn't a valid PDF (it was probably an error page)."
                        .to_string(),
                )
            }
        }
        "epub" => {
            if head.starts_with(b"PK\x03\x04") {
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

/// `Retry-After` in its delta-seconds form.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
}

/// Sleep that returns `false` early if `cancel` fires.
async fn nap(cancel: &CancellationToken, d: std::time::Duration) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(d) => true,
    }
}

/// GET a document URL with document-shaped headers and bounded, cancel-aware
/// retry on 429/5xx/transport errors. Feeds the per-host limiter: success
/// widens the host's window, 429/503 halves it and honors `Retry-After`.
async fn get_document_response(
    client: &reqwest::Client,
    url: &str,
    cancel: &CancellationToken,
    limiter: &HostLimiter,
    host: &str,
) -> Result<reqwest::Response, DownloadOutcome> {
    const ATTEMPTS: u32 = 3;
    const MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
    let referer = referer_for(url);
    for attempt in 0..ATTEMPTS {
        let mut req = client
            .get(url)
            .header(reqwest::header::ACCEPT, DOC_ACCEPT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        if let Some(r) = &referer {
            req = req.header(reqwest::header::REFERER, r.clone());
        }
        // Per-host UA override: Zenodo's WAF 403s the client's default browser
        // UA but serves an honest tool UA normally (search + file downloads).
        if host_needs_honest_ua(url) {
            req = req.header(reqwest::header::USER_AGENT, REPO_UA);
        }
        let sent = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DownloadOutcome::Cancelled),
            r = req.send() => r,
        };
        let backoff = std::time::Duration::from_secs(2 * (attempt as u64 + 1));
        match sent {
            Ok(r) if r.status().is_success() => {
                limiter.reward(host);
                return Ok(r);
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let ra = retry_after(r.headers());
                if status == 429 || status == 503 {
                    limiter.penalize(host, ra);
                }
                if matches!(status, 401 | 403 | 451) {
                    // Charge the host that actually refused (after redirects),
                    // not a resolver like doi.org that merely forwarded us.
                    limiter.record_refusal(&super::hostlimit::host_of(r.url().as_str()));
                }
                let retryable = status == 429 || (500..=599).contains(&status);
                if attempt + 1 < ATTEMPTS && retryable {
                    if !nap(cancel, ra.unwrap_or(backoff).min(MAX_WAIT)).await {
                        return Err(DownloadOutcome::Cancelled);
                    }
                    continue;
                }
                return Err(DownloadOutcome::Failed(format!(
                    "{}. (HTTP {status})",
                    http_status_message(status)
                )));
            }
            Err(e) if attempt + 1 < ATTEMPTS && !e.is_builder() && !e.is_redirect() => {
                if !nap(cancel, backoff).await {
                    return Err(DownloadOutcome::Cancelled);
                }
            }
            Err(e) => return Err(DownloadOutcome::Failed(network_error_message(&e))),
        }
    }
    Err(DownloadOutcome::Failed(
        "Download failed after multiple attempts.".to_string(),
    ))
}

type ByteStream = futures::stream::BoxStream<'static, reqwest::Result<bytes::Bytes>>;

/// Pull the next chunk, racing a cancel. `Err(())` = cancelled.
async fn next_chunk(
    stream: &mut ByteStream,
    cancel: &CancellationToken,
) -> Result<Option<reqwest::Result<bytes::Bytes>>, ()> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(()),
        c = stream.next() => Ok(c),
    }
}

/// Valid existing copy of this slug in `dest_dir` (any document extension).
/// Stale junk under a slot is removed so a fresh download has a clean path.
pub async fn existing_copy(dest_dir: &Path, slug: &str) -> Option<PathBuf> {
    const RESUME_EXTS: [&str; 4] = [".pdf", ".epub", ".html", ".txt"];
    for ext in RESUME_EXTS {
        let candidate = dest_dir.join(format!("{slug}{ext}"));
        let Ok(meta) = tokio::fs::symlink_metadata(&candidate).await else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.len() >= MIN_DOC_BYTES && existing_file_is_valid(&candidate).await {
            return Some(candidate);
        }
        let _ = tokio::fs::remove_file(&candidate).await;
    }
    None
}

/// Download `doc`, trying its primary URL, then each merged/alternate URL,
/// then Unpaywall's open-access locations for its DOI. A valid copy already
/// in `dest_dir` is reused (`Cached`, no network). The first failure's
/// message is reported if every route fails.
pub async fn download<F>(
    doc: &Document,
    dest_dir: &Path,
    client: &reqwest::Client,
    limiter: &Arc<HostLimiter>,
    cancel: &CancellationToken,
    mut on_progress: F,
) -> DownloadOutcome
where
    F: FnMut(ProgressEvent) + Send,
{
    /// Bound on routes tried per document so one stubborn doc can't hog a worker.
    const MAX_ROUTES: usize = 5;
    let slug = doc.slug();
    if let Some(p) = existing_copy(dest_dir, &slug).await {
        return DownloadOutcome::Cached(p);
    }
    let mut tried: Vec<String> = Vec::new();
    let mut first_err: Option<String> = None;
    let mut routes: Vec<String> = doc.all_urls();
    let mut asked_unpaywall = false;
    let mut i = 0;
    loop {
        if i >= routes.len() {
            // Out of known URLs: ask Unpaywall once for more (DOI-bearing docs).
            if asked_unpaywall || tried.len() >= MAX_ROUTES {
                break;
            }
            asked_unpaywall = true;
            let Some(doi) = doc.best_doi() else { break };
            let found = tokio::select! {
                biased;
                _ = cancel.cancelled() => return DownloadOutcome::Cancelled,
                r = resolve_oa_pdfs_via_unpaywall(client, &doi) => r,
            };
            let before = routes.len();
            for u in found {
                if !routes.contains(&u) {
                    routes.push(u);
                }
            }
            if routes.len() == before {
                break;
            }
            continue;
        }
        let url = canonicalize_doc_url(&routes[i]);
        i += 1;
        if tried.contains(&url) {
            continue;
        }
        if tried.len() >= MAX_ROUTES {
            break;
        }
        if limiter.is_blocked(&super::hostlimit::host_of(&url)) {
            first_err.get_or_insert_with(|| "This source blocks automated downloads.".to_string());
            continue;
        }
        tried.push(url.clone());
        match download_url(
            &slug,
            &url,
            dest_dir,
            client,
            limiter,
            cancel,
            &mut on_progress,
        )
        .await
        {
            DownloadOutcome::Failed(e) => {
                tracing::debug!("route failed for {}: {url}: {e}", doc.title);
                first_err.get_or_insert(e);
            }
            other => return other,
        }
        if cancel.is_cancelled() {
            return DownloadOutcome::Cancelled;
        }
    }
    DownloadOutcome::Failed(
        first_err.unwrap_or_else(|| "No downloadable link was found for this document.".into()),
    )
}

/// One route: fetch `url` (following up to two landing-page → PDF hops) and
/// stream it into `<slug>.<ext>` atomically.
#[allow(clippy::too_many_arguments)]
async fn download_url<F>(
    slug: &str,
    url: &str,
    dest_dir: &Path,
    client: &reqwest::Client,
    limiter: &Arc<HostLimiter>,
    cancel: &CancellationToken,
    on_progress: &mut F,
) -> DownloadOutcome
where
    F: FnMut(ProgressEvent) + Send,
{
    const MAX_HOPS: usize = 2;
    let mut target = url.to_string();
    let mut hops = 0usize;

    // Resolve to a document body: (permit, prefix bytes already read, rest of
    // the stream, content-type, content-length).
    let (_permit, prefix, mut stream, content_type, total) = loop {
        if let Err(reason) = crate::util::url_safety::validate_download_url(&target).await {
            return DownloadOutcome::Failed(format!("Blocked for safety: {reason}"));
        }
        let host = super::hostlimit::host_of(&target);
        let Some(permit) = limiter.acquire(&host, cancel).await else {
            return DownloadOutcome::Cancelled;
        };
        let resp = match get_document_response(client, &target, cancel, limiter, &host).await {
            Ok(r) => r,
            Err(outcome) => return outcome,
        };
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let total = resp.content_length().unwrap_or(0);
        if reject_content_type(&target, &content_type).is_none() {
            break (
                permit,
                Vec::new(),
                resp.bytes_stream().boxed(),
                content_type,
                total,
            );
        }

        // Looks like a web page. Peek the first KB: some OA endpoints serve a
        // real PDF/EPUB under `text/html` — if so, keep streaming THIS response
        // (no second request) instead of re-fetching it.
        let base = resp.url().clone();
        let mut stream: ByteStream = resp.bytes_stream().boxed();
        let mut buf: Vec<u8> = Vec::new();
        while buf.len() < 1024 {
            match next_chunk(&mut stream, cancel).await {
                Err(()) => return DownloadOutcome::Cancelled,
                Ok(Some(Ok(c))) => buf.extend_from_slice(&c),
                Ok(_) => break,
            }
        }
        if sniff_doc_ext(&buf[..buf.len().min(1024)]).is_some() {
            break (permit, buf, stream, String::new(), total);
        }
        // A real page: read (bounded) and hunt for the PDF link.
        while buf.len() < HTML_RESOLVE_CAP {
            match next_chunk(&mut stream, cancel).await {
                Err(()) => return DownloadOutcome::Cancelled,
                Ok(Some(Ok(c))) => buf.extend_from_slice(&c),
                Ok(_) => break,
            }
        }
        drop(stream);
        drop(permit);
        if hops < MAX_HOPS {
            if let Some(pdf_url) = resolve_pdf_from_html(&String::from_utf8_lossy(&buf), &base) {
                let pdf_url = canonicalize_doc_url(&pdf_url);
                if pdf_url != target && pdf_url != base.as_str() {
                    target = pdf_url;
                    hops += 1;
                    continue;
                }
            }
        }
        limiter.record_refusal(&super::hostlimit::host_of(base.as_str()));
        return DownloadOutcome::Failed(
            "This link opened a web page, not a downloadable document.".to_string(),
        );
    };

    let guessed_ext = ext_from(&target, &content_type);
    if total > MAX_FILE_BYTES {
        return DownloadOutcome::Failed(format!(
            "This file is too large to download (over {} MB).",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }

    // Stream into a sibling `.part` temp, then atomically rename only after
    // the bytes pass validation, so the final path never holds a truncated
    // body. Buffered (256 KB) so small network chunks don't each cost a
    // blocking-pool hop.
    let tmp = dest_dir.join(format!("{slug}.part"));
    let file = match File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => return DownloadOutcome::Failed(format!("Couldn't create the file on disk: {e}")),
    };
    let mut file = tokio::io::BufWriter::with_capacity(256 * 1024, file);
    let mut head: Vec<u8> = Vec::with_capacity(1024);
    let mut downloaded: u64 = 0;
    let started = std::time::Instant::now();
    let fail = |msg: String| async {
        let _ = tokio::fs::remove_file(&tmp).await;
        DownloadOutcome::Failed(msg)
    };

    let mut pending: Option<bytes::Bytes> = if prefix.is_empty() {
        None
    } else {
        Some(bytes::Bytes::from(prefix))
    };
    loop {
        let chunk = match pending.take() {
            Some(c) => c,
            None => match next_chunk(&mut stream, cancel).await {
                Err(()) => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return DownloadOutcome::Cancelled;
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    drop(file);
                    return fail(network_error_message(&e)).await;
                }
                Ok(Some(Ok(c))) => c,
            },
        };
        if chunk.is_empty() {
            continue;
        }
        if head.len() < 1024 {
            let take = (1024 - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            return fail(format!("Couldn't save the file to disk: {e}")).await;
        }
        downloaded += chunk.len() as u64;
        if downloaded > MAX_FILE_BYTES {
            drop(file);
            return fail(format!(
                "This file is too large to download (over {} MB).",
                MAX_FILE_BYTES / 1024 / 1024
            ))
            .await;
        }
        on_progress(ProgressEvent {
            downloaded,
            total,
            force: false,
        });
        // Cap slow transfers: a stalled or crawling server must not hold a
        // worker (and the end of the run) hostage.
        let elapsed = started.elapsed();
        if elapsed > MAX_TRANSFER {
            drop(file);
            return fail(SLOW_MSG.to_string()).await;
        }
        if elapsed > SLOW_GRACE {
            let rate = downloaded as f64 / elapsed.as_secs_f64();
            let projected_too_long = total > downloaded
                && rate > 0.0
                && (total - downloaded) as f64 / rate > MAX_PROJECTED.as_secs_f64();
            if rate < MIN_RATE_BPS || projected_too_long {
                drop(file);
                return fail(SLOW_MSG.to_string()).await;
            }
        }
    }
    on_progress(ProgressEvent {
        downloaded,
        total: if total == 0 { downloaded } else { total },
        force: true,
    });
    if let Err(e) = file.flush().await {
        drop(file);
        return fail(format!("Couldn't finish saving the file to disk: {e}")).await;
    }
    drop(file);

    let tmp_meta = match tokio::fs::symlink_metadata(&tmp).await {
        Ok(m) => m,
        Err(e) => return fail(format!("Couldn't read the saved file: {e}")).await,
    };
    if tmp_meta.file_type().is_symlink() {
        return fail("The saved file was unexpectedly replaced.".to_string()).await;
    }
    let size = tmp_meta.len();
    if size == 0 {
        return fail("The server returned no data.".to_string()).await;
    }
    if size < MIN_DOC_BYTES {
        return fail(format!(
            "The server returned only {size} bytes — likely an error page, not the document."
        ))
        .await;
    }
    let ct_lower = content_type.to_lowercase();
    let url_lower = target.to_lowercase();
    let url_path = url_lower.split('?').next().unwrap_or(&url_lower);
    let claimed_binary = ct_lower.contains("pdf")
        || ct_lower.contains("epub")
        || url_path.ends_with(".pdf")
        || url_path.ends_with(".epub");
    let real_ext = match reconcile_doc_ext(&head, guessed_ext, claimed_binary) {
        Ok(e) => e,
        Err(()) => {
            return fail(
                "The downloaded file wasn't a valid document (it was probably an error page)."
                    .to_string(),
            )
            .await
        }
    };
    let out = dest_dir.join(format!("{slug}{real_ext}"));
    if let Err(e) = tokio::fs::rename(&tmp, &out).await {
        return fail(format!("Couldn't finalize the download: {e}")).await;
    }
    limiter.record_ok(&super::hostlimit::host_of(&target));
    DownloadOutcome::Saved(out)
}

/// Copy a previously downloaded file (found via the library-wide index) into
/// `dest_dir` as `<slug>.<ext>`, if it still passes the same validation a
/// fresh download would. Symlinks are never followed.
pub async fn reuse_file(src: &Path, dest_dir: &Path, slug: &str) -> Option<PathBuf> {
    let meta = tokio::fs::symlink_metadata(src).await.ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    if meta.len() < MIN_DOC_BYTES || !existing_file_is_valid(src).await {
        return None;
    }
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("pdf");
    let dest = dest_dir.join(format!("{slug}.{ext}"));
    if dest == src {
        return Some(dest);
    }
    tokio::fs::copy(src, &dest).await.ok()?;
    Some(dest)
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

    #[test]
    fn resolves_data_pdf_url_attribute() {
        let base = url::Url::parse("https://repo.edu/record/9").unwrap();
        let html =
            r#"<button class="dl" data-pdf-url="/record/9/files/paper.pdf">Download</button>"#;
        assert_eq!(
            resolve_pdf_from_html(html, &base).as_deref(),
            Some("https://repo.edu/record/9/files/paper.pdf")
        );
    }

    #[test]
    fn resolves_same_host_download_endpoint_without_pdf_extension() {
        let base = url::Url::parse("https://ojs.example.org/index.php/j/article/view/10").unwrap();
        let html =
            r#"<a class="obj_galley_link" href="/index.php/j/article/download/10/9">PDF</a>"#;
        assert_eq!(
            resolve_pdf_from_html(html, &base).as_deref(),
            Some("https://ojs.example.org/index.php/j/article/download/10/9")
        );
    }

    #[test]
    fn skips_cross_host_download_endpoint() {
        let base = url::Url::parse("https://repo.edu/record/9").unwrap();
        let html = r#"<a href="https://ads.example.com/download/promo">grab</a>"#;
        assert_eq!(resolve_pdf_from_html(html, &base), None);
    }

    fn test_doc(title: &str, url: &str) -> Document {
        Document {
            title: title.into(),
            url: url.into(),
            source: "arxiv".into(),
            authors: vec![],
            year: None,
            abstract_: None,
            identifier: None,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn reuse_copies_a_valid_indexed_file() {
        let root = tempfile::tempdir().unwrap();
        let lib_a = root.path().join("query-a");
        let lib_b = root.path().join("query-b");
        tokio::fs::create_dir_all(&lib_a).await.unwrap();
        tokio::fs::create_dir_all(&lib_b).await.unwrap();
        let doc = test_doc("Shared Paper", "https://example.org/shared.pdf");
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(9000, b' ');
        let src = lib_a.join(format!("{}.pdf", doc.slug()));
        tokio::fs::write(&src, &pdf).await.unwrap();
        let dest = reuse_file(&src, &lib_b, &doc.slug()).await.unwrap();
        assert!(dest.starts_with(&lib_b));
        assert!(tokio::fs::metadata(&dest).await.is_ok());
    }

    #[tokio::test]
    async fn reuse_rejects_invalid_or_missing_files() {
        let root = tempfile::tempdir().unwrap();
        let doc = test_doc("Junk", "https://example.org/junk.pdf");
        let missing = root.path().join("nope.pdf");
        assert!(reuse_file(&missing, root.path(), &doc.slug())
            .await
            .is_none());
        let mut junk = b"<!DOCTYPE html><html>nope</html>".to_vec();
        junk.resize(9000, b' ');
        let src = root.path().join("junk.pdf");
        tokio::fs::write(&src, &junk).await.unwrap();
        let out = root.path().join("out");
        tokio::fs::create_dir_all(&out).await.unwrap();
        assert!(reuse_file(&src, &out, &doc.slug()).await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reuse_never_follows_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let doc = test_doc("Linked", "https://example.org/linked.pdf");
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(9000, b' ');
        let target = root.path().join("real-target.pdf");
        tokio::fs::write(&target, &pdf).await.unwrap();
        let link = root.path().join("link.pdf");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let out = root.path().join("out");
        tokio::fs::create_dir_all(&out).await.unwrap();
        assert!(reuse_file(&link, &out, &doc.slug()).await.is_none());
    }

    #[test]
    fn rewrites_known_landing_pages() {
        assert_eq!(
            canonicalize_doc_url("https://www.ncbi.nlm.nih.gov/pmc/articles/PMC1234567/"),
            "https://europepmc.org/backend/ptpmcrender.fcgi?accid=PMC1234567&blobtype=pdf"
        );
        assert_eq!(
            canonicalize_doc_url("https://www.biorxiv.org/content/10.1101/2020.01.01.123456v2"),
            "https://www.biorxiv.org/content/10.1101/2020.01.01.123456v2.full.pdf"
        );
        assert_eq!(
            canonicalize_doc_url("https://osf.io/ab12c/"),
            "https://osf.io/ab12c/download"
        );
        assert_eq!(
            canonicalize_doc_url("https://doi.org/10.48550/arXiv.2101.00001"),
            "https://arxiv.org/pdf/2101.00001"
        );
        assert_eq!(
            canonicalize_doc_url("https://example.com/x.pdf"),
            "https://example.com/x.pdf"
        );
    }

    #[tokio::test]
    async fn download_reuses_existing_copy_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let doc = test_doc("Already Here", "https://invalid.invalid/a.pdf");
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(9000, b' ');
        tokio::fs::write(dir.path().join(format!("{}.pdf", doc.slug())), &pdf)
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let limiter = HostLimiter::new();
        let cancel = CancellationToken::new();
        let out = download(&doc, dir.path(), &client, &limiter, &cancel, |_| {}).await;
        assert!(matches!(out, DownloadOutcome::Cached(_)));
    }

    #[tokio::test]
    async fn check_magic_bytes_accepts_bom_prefixed_pdf() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.pdf");
        let mut f = tokio::fs::File::create(&path).await.unwrap();
        // A BOM / leading bytes before %PDF is legitimate and must be accepted.
        f.write_all(b"\xef\xbb\xbf%PDF-1.7\n").await.unwrap();
        f.write_all(&[b' '; 2048]).await.unwrap();
        f.flush().await.unwrap();
        drop(f);
        assert!(
            check_magic_bytes(&path).await.is_none(),
            "BOM-prefixed PDF should pass magic-byte validation"
        );
    }

    #[tokio::test]
    async fn check_magic_bytes_rejects_html_named_pdf() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.pdf");
        let mut f = tokio::fs::File::create(&path).await.unwrap();
        f.write_all(b"<!DOCTYPE html><html>error page</html>")
            .await
            .unwrap();
        f.write_all(&[b' '; 2048]).await.unwrap();
        f.flush().await.unwrap();
        drop(f);
        assert!(
            check_magic_bytes(&path).await.is_some(),
            "an HTML error page saved as .pdf must be rejected"
        );
    }
}
