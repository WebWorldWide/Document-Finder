//! Plain text extraction utilities for supported document formats (PDF, EPUB, HTML, TXT).

use once_cell::sync::Lazy;
use regex::Regex;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

pub fn extract_text(path: &Path) -> anyhow::Result<String> {
    let suffix = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match suffix.as_str() {
        "txt" => std::fs::read_to_string(path).map_err(anyhow::Error::from),
        "pdf" => extract_pdf(path),
        "epub" => extract_epub(path),
        "html" | "htm" => std::fs::read_to_string(path)
            .map(|h| strip_html(&h))
            .map_err(anyhow::Error::from),
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

fn extract_epub_inner(path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut chunks: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_lowercase();
        if !(name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm")) {
            continue;
        }
        let mut buf = String::new();
        if entry.read_to_string(&mut buf).is_ok() {
            chunks.push(strip_html(&buf));
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
