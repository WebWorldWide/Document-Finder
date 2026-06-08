//! Plain text extraction utilities for supported document formats (PDF, EPUB, HTML, TXT).

use once_cell::sync::Lazy;
use regex::Regex;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

/// Cap on raw source text (TXT/HTML) read into memory before extraction. With
/// up to `num_cpus` extractions running concurrently, an unbounded read of a
/// pathological multi-hundred-MB file (times N) could exhaust memory.
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

/// Read a text file as lossy UTF-8, bounded to `MAX_TEXT_BYTES`. `read_to_string`
/// hard-errors on any non-UTF-8 byte (Latin-1, UTF-16, legacy encodings), which
/// would silently drop otherwise-readable documents; lossy decoding keeps the
/// readable text instead.
fn read_text_file_capped(path: &Path) -> anyhow::Result<String> {
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(MAX_TEXT_BYTES).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

pub fn extract_text(path: &Path) -> anyhow::Result<String> {
    let suffix = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match suffix.as_str() {
        "txt" => read_text_file_capped(path),
        "pdf" => extract_pdf(path),
        "epub" => extract_epub(path),
        "html" | "htm" => read_text_file_capped(path).map(|h| strip_html(&h)),
        _ => Err(anyhow::anyhow!("Unsupported file extension: .{}", suffix)),
    }
}

// `pdf_extract` (and its `lopdf` backing) regularly panics on malformed PDFs.
// Catch the unwind so a single bad file never aborts the run.
fn extract_pdf(path: &Path) -> anyhow::Result<String> {
    let p: PathBuf = path.to_path_buf();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text(&p)));
    match result {
        Ok(Ok(s)) if !s.trim().is_empty() => Ok(s),
        Ok(Ok(_)) => Err(anyhow::anyhow!("PDF is empty")),
        Ok(Err(e)) => Err(anyhow::anyhow!("pdf extraction failed: {}", e)),
        Err(_) => Err(anyhow::anyhow!("pdf_extract panicked")),
    }
}

fn extract_epub(path: &Path) -> anyhow::Result<String> {
    let p: PathBuf = path.to_path_buf();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| extract_epub_inner(&p)));
    match result {
        Ok(res) => res,
        Err(_) => Err(anyhow::anyhow!("epub extraction panicked")),
    }
}

/// Hard caps to defuse zip/decompression bombs: a small EPUB can otherwise
/// declare a multi-GB uncompressed entry. We skip any entry whose declared
/// uncompressed size exceeds the per-entry cap, read at most that many bytes,
/// and stop once the accumulated text passes the total cap.
const MAX_EPUB_ENTRY_BYTES: u64 = 64 * 1024 * 1024; // 64 MB per HTML entry
const MAX_EPUB_TOTAL_BYTES: usize = 128 * 1024 * 1024; // 128 MB of text total

fn extract_epub_inner(path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut chunks: Vec<String> = Vec::new();
    let mut total: usize = 0;
    for i in 0..zip.len() {
        if total >= MAX_EPUB_TOTAL_BYTES {
            break;
        }
        let entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_lowercase();
        if !(name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm")) {
            continue;
        }
        // Skip entries whose declared uncompressed size is implausibly large…
        if entry.size() > MAX_EPUB_ENTRY_BYTES {
            continue;
        }
        // …and still cap the actual read — bounded by the *remaining* total
        // budget, not just the per-entry cap, so a final oversized entry can't
        // push the buffered text past MAX_EPUB_TOTAL_BYTES (the loop only
        // re-checks `total` between entries, so the cap was previously a soft
        // limit that one extra ~64 MB entry could overshoot).
        let remaining = MAX_EPUB_TOTAL_BYTES.saturating_sub(total) as u64;
        if remaining == 0 {
            break;
        }
        let read_cap = remaining.min(MAX_EPUB_ENTRY_BYTES);
        let mut buf = Vec::new();
        if entry.take(read_cap).read_to_end(&mut buf).is_ok() {
            let html_str = String::from_utf8_lossy(&buf);
            let stripped = strip_html(&html_str);
            total += stripped.len();
            chunks.push(stripped);
        }
    }
    if chunks.is_empty() {
        Err(anyhow::anyhow!("EPUB contains no text content"))
    } else {
        Ok(chunks.join("\n\n"))
    }
}

static SCRIPT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<\s*script\b[^>]*>.*?<\s*/\s*script\s*>").unwrap());
static STYLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<\s*style\b[^>]*>.*?<\s*/\s*style\s*>").unwrap());
static NOSCRIPT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<\s*noscript\b[^>]*>.*?<\s*/\s*noscript\s*>").unwrap());
static BLOCK_OPEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)<\s*(p|div|br|h[1-6]|li|tr|section|article|header|footer|hr|blockquote)\b[^>]*/?>",
    )
    .unwrap()
});
static BLOCK_CLOSE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)<\s*/\s*(p|div|h[1-6]|li|tr|section|article|header|footer|blockquote)\s*>")
        .unwrap()
});
static ANY_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
static ENTITY: Lazy<Regex> = Lazy::new(|| Regex::new(r"&(amp|lt|gt|quot|nbsp|#39|apos);").unwrap());

pub fn strip_html(html: &str) -> String {
    let s = SCRIPT_RE.replace_all(html, "");
    let s = STYLE_RE.replace_all(&s, "").to_string();
    let s = NOSCRIPT_RE.replace_all(&s, "").to_string();
    let with_breaks = BLOCK_OPEN.replace_all(&s, "\n");
    let with_breaks = BLOCK_CLOSE.replace_all(&with_breaks, "\n");
    let no_tags = ANY_TAG.replace_all(&with_breaks, "");
    let decoded = ENTITY.replace_all(&no_tags, |caps: &regex::Captures| {
        match &caps[1] {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "nbsp" => " ",
            "#39" | "apos" => "'",
            _ => "",
        }
        .to_string()
    });
    decoded.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_removes_scripts() {
        let html = "<html><body><p>hello</p><script>alert(1)</script><p>world</p></body></html>";
        let out = strip_html(html);
        assert!(!out.contains("alert"));
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
    }

    #[test]
    fn strip_html_decodes_entities() {
        assert_eq!(strip_html("<p>foo &amp; bar</p>").trim(), "foo & bar");
    }
}
