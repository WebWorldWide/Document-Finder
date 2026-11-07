//! Text extraction for downloaded documents (PDF, EPUB, HTML, TXT) into
//! clean Markdown plus retrieval chunks.
//!
//! * **PDF** — PDFium (Chrome's PDF engine, bundled as a native library) gives
//!   per-page text in reading order; `pdf-extract` is the pure-Rust fallback
//!   when PDFium can't be loaded or fails on a file.
//! * **EPUB** — chapters in OPF *spine* order (the real reading order), with
//!   the navigation document skipped.
//! * **HTML** — main-content extraction to Markdown (see `html2md`).
//! * **TXT** — encoding detection (UTF-8/UTF-16/legacy code pages).
//!
//! All formats then go through `textclean` (ligatures, running headers,
//! page numbers, hyphenation, headings) and are chunked for full-text search.

use super::html2md::html_to_markdown;
use super::textclean::{chunk_markdown, detect_language, estimate_tokens, pages_to_markdown};
use crate::engine::db::ChunkRecord;
use once_cell::sync::OnceCell;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

/// Cap on raw source text (TXT/HTML) read into memory before extraction.
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

/// Cap on PDF input we extract. The downloader accepts up to 500 MB; huge
/// scans are skipped with an error rather than risking memory spikes.
const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;

/// Per-entry / total caps against EPUB zip bombs.
const MAX_EPUB_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EPUB_TOTAL_BYTES: usize = 128 * 1024 * 1024;

/// Below this many characters per page a PDF is almost certainly a scan with
/// no text layer.
const SCANNED_CHARS_PER_PAGE: usize = 40;

/// The result of extracting one document.
#[derive(Debug, Clone, Default)]
pub struct Extracted {
    /// Clean Markdown body (no front matter — the caller adds metadata).
    pub markdown: String,
    pub pages: Option<u32>,
    pub lang: Option<String>,
    pub words: usize,
    pub tokens: usize,
    pub chunks: Vec<ChunkRecord>,
    /// Which engine produced the text (for diagnostics).
    pub engine: &'static str,
}

/// Directories searched for the bundled PDFium library, set once at startup
/// from the app's resource dir. The executable's own directory and the
/// system library path are always tried too.
static PDFIUM_DIRS: OnceCell<Vec<PathBuf>> = OnceCell::new();
static PDFIUM: OnceCell<Option<pdfium_render::prelude::Pdfium>> = OnceCell::new();

pub fn set_pdfium_dirs(dirs: Vec<PathBuf>) {
    let _ = PDFIUM_DIRS.set(dirs);
}

fn pdfium() -> Option<&'static pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::Pdfium;
    PDFIUM
        .get_or_init(|| {
            let mut dirs: Vec<PathBuf> = PDFIUM_DIRS.get().cloned().unwrap_or_default();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(d) = exe.parent() {
                    dirs.push(d.to_path_buf());
                    // macOS bundles it in Contents/Frameworks (signed with the app).
                    dirs.push(d.join("../Frameworks"));
                    dirs.push(d.join("../Resources/pdfium"));
                    dirs.push(d.join("../lib/Document Finder/pdfium"));
                    dirs.push(d.join("pdfium"));
                }
            }
            if let Ok(d) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
                dirs.insert(0, PathBuf::from(d));
            }
            for d in &dirs {
                let lib = Pdfium::pdfium_platform_library_name_at_path(d);
                if !lib.exists() {
                    continue;
                }
                match Pdfium::bind_to_library(&lib) {
                    Ok(b) => {
                        tracing::info!("PDFium loaded from {}", lib.display());
                        return Some(Pdfium::new(b));
                    }
                    Err(e) => tracing::warn!("PDFium at {} failed to load: {e}", lib.display()),
                }
            }
            match Pdfium::bind_to_system_library() {
                Ok(b) => Some(Pdfium::new(b)),
                Err(_) => {
                    tracing::warn!("PDFium not found — falling back to pdf-extract");
                    None
                }
            }
        })
        .as_ref()
}

/// True when the bundled PDF engine is available (for diagnostics/UI).
pub fn pdfium_available() -> bool {
    pdfium().is_some()
}

/// Extract `path` into Markdown + chunks. `title` labels chunks that come
/// before the first detected section heading.
pub fn extract(path: &Path, title: &str) -> anyhow::Result<Extracted> {
    let suffix = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (markdown, pages, engine) = match suffix.as_str() {
        "pdf" => extract_pdf(path)?,
        "epub" => (guard("EPUB", || extract_epub(path))?, None, "epub"),
        "html" | "htm" => {
            let html = read_text_file_capped(path)?;
            let md = html_to_markdown(&html);
            (pages_to_markdown(&[md]).markdown, None, "html")
        }
        "txt" | "md" => {
            let text = read_text_file_capped(path)?;
            (pages_to_markdown(&[text]).markdown, None, "text")
        }
        _ => return Err(anyhow::anyhow!("Unsupported file type: .{suffix}")),
    };
    let markdown = truncate_text(markdown);
    let body_chars = markdown
        .lines()
        .filter(|l| !l.starts_with("<!--"))
        .map(|l| l.trim().len())
        .sum::<usize>();
    if body_chars == 0 {
        return Err(anyhow::anyhow!("No readable text found in this file"));
    }
    if let Some(p) = pages {
        if p > 0 && body_chars / (p as usize) < SCANNED_CHARS_PER_PAGE {
            return Err(anyhow::anyhow!(
                "This PDF is a scanned image with no text layer (OCR isn't supported yet)"
            ));
        }
    }
    let words = markdown.split_whitespace().count();
    let chunks = chunk_markdown(&markdown, title);
    Ok(Extracted {
        lang: detect_language(&markdown),
        tokens: estimate_tokens(&markdown),
        words,
        chunks,
        pages,
        markdown,
        engine,
    })
}

fn guard<T>(what: &str, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!("{what} extraction crashed on this file")),
    }
}

fn truncate_text(mut s: String) -> String {
    let cap = MAX_TEXT_BYTES as usize;
    if s.len() > cap {
        let mut end = cap;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Read a text file, detecting its encoding (BOM, then a statistical guess),
/// bounded to [`MAX_TEXT_BYTES`].
fn read_text_file_capped(path: &Path) -> anyhow::Result<String> {
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(MAX_TEXT_BYTES).read_to_end(&mut buf)?;
    Ok(decode_bytes(&buf))
}

pub fn decode_bytes(buf: &[u8]) -> String {
    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(buf) {
        return enc
            .decode_without_bom_handling(&buf[bom_len..])
            .0
            .into_owned();
    }
    if let Ok(s) = std::str::from_utf8(buf) {
        return s.to_string();
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(buf, true);
    let enc = det.guess(None, true);
    enc.decode_without_bom_handling(buf).0.into_owned()
}

fn extract_pdf(path: &Path) -> anyhow::Result<(String, Option<u32>, &'static str)> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_PDF_BYTES {
            return Err(anyhow::anyhow!(
                "PDF too large to extract ({} MB, limit {} MB)",
                meta.len() / 1024 / 1024,
                MAX_PDF_BYTES / 1024 / 1024
            ));
        }
    }
    let mut pdfium_err: Option<anyhow::Error> = None;
    if let Some(engine) = pdfium() {
        match guard("PDF", || pdfium_pages(engine, path)) {
            Ok(pages) => {
                let n = pages.len() as u32;
                let md = pages_to_markdown(&pages).markdown;
                return Ok((md, Some(n), "pdfium"));
            }
            Err(e) => pdfium_err = Some(e),
        }
    }
    // Fallback: pure-Rust pdf-extract (form feeds separate pages).
    let p = path.to_path_buf();
    match std::panic::catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text(&p))) {
        Ok(Ok(s)) if !s.trim().is_empty() => {
            let pages: Vec<String> = s.split('\x0c').map(String::from).collect();
            let n = pages.len() as u32;
            Ok((pages_to_markdown(&pages).markdown, Some(n), "pdf-extract"))
        }
        Ok(Ok(_)) => Err(pdfium_err.unwrap_or_else(|| {
            anyhow::anyhow!(
                "This PDF is a scanned image with no text layer (OCR isn't supported yet)"
            )
        })),
        Ok(Err(e)) => {
            Err(pdfium_err.unwrap_or_else(|| anyhow::anyhow!("Couldn't read this PDF: {e}")))
        }
        Err(_) => Err(pdfium_err.unwrap_or_else(|| anyhow::anyhow!("Couldn't read this PDF"))),
    }
}

fn pdfium_pages(
    engine: &pdfium_render::prelude::Pdfium,
    path: &Path,
) -> anyhow::Result<Vec<String>> {
    let doc = engine
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("Couldn't open this PDF: {e}"))?;
    let mut pages = Vec::new();
    for page in doc.pages().iter() {
        let text = page.text().map(|t| t.all()).unwrap_or_default();
        pages.push(text);
    }
    Ok(pages)
}

// ---------------------------------------------------------------- EPUB ----

fn zip_read(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, cap: u64) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    if entry.size() > MAX_EPUB_ENTRY_BYTES {
        return None;
    }
    let mut buf = Vec::new();
    entry.take(cap).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=");
    // Match a whole attribute name (preceded by whitespace), so `id=` can't
    // hit inside `idref=` / `xid=`.
    let i = tag
        .match_indices(&pat)
        .find(|(i, _)| *i > 0 && tag.as_bytes()[i - 1].is_ascii_whitespace())?
        .0
        + pat.len();
    let rest = &tag[i..];
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let end = rest[1..].find(q)?;
    Some(&rest[1..1 + end])
}

/// Spine-ordered chapter paths from an OPF package document.
fn spine_paths(opf: &str, opf_dir: &str) -> Vec<String> {
    let mut manifest: std::collections::HashMap<String, (String, bool)> = Default::default();
    for tag in opf
        .split('<')
        .filter(|t| t.starts_with("item ") || t.starts_with("opf:item "))
    {
        let (Some(id), Some(href)) = (attr(tag, "id"), attr(tag, "href")) else {
            continue;
        };
        let is_nav = attr(tag, "properties")
            .map(|p| p.contains("nav"))
            .unwrap_or(false);
        manifest.insert(id.to_string(), (href.to_string(), is_nav));
    }
    let mut out = Vec::new();
    for tag in opf
        .split('<')
        .filter(|t| t.starts_with("itemref") || t.starts_with("opf:itemref"))
    {
        let Some(idref) = attr(tag, "idref") else {
            continue;
        };
        if attr(tag, "linear") == Some("no") {
            continue;
        }
        if let Some((href, is_nav)) = manifest.get(idref) {
            if *is_nav {
                continue;
            }
            let href = percent_encoding::percent_decode_str(href)
                .decode_utf8_lossy()
                .to_string();
            let full = if opf_dir.is_empty() {
                href
            } else {
                format!("{opf_dir}/{href}")
            };
            out.push(full);
        }
    }
    out
}

fn extract_epub(path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    // META-INF/container.xml → OPF path → spine order.
    let opf_path = zip_read(&mut zip, "META-INF/container.xml", 1 << 20)
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .and_then(|c| {
            c.split('<')
                .filter(|t| t.starts_with("rootfile ") || t.starts_with("rootfile\n"))
                .find_map(|t| attr(t, "full-path").map(String::from))
        });
    let mut order: Vec<String> = Vec::new();
    if let Some(opf_path) = &opf_path {
        if let Some(opf) = zip_read(&mut zip, opf_path, 8 << 20) {
            let dir = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            order = spine_paths(&String::from_utf8_lossy(&opf), dir);
        }
    }
    if order.is_empty() {
        // No usable spine: fall back to every (X)HTML entry in archive order.
        order = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
            .filter(|n| {
                let l = n.to_lowercase();
                (l.ends_with(".xhtml") || l.ends_with(".html") || l.ends_with(".htm"))
                    && !l.contains("nav.")
                    && !l.contains("toc.")
            })
            .collect();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut total = 0usize;
    for name in order {
        if total >= MAX_EPUB_TOTAL_BYTES {
            break;
        }
        let remaining = (MAX_EPUB_TOTAL_BYTES - total) as u64;
        let Some(buf) = zip_read(&mut zip, &name, remaining.min(MAX_EPUB_ENTRY_BYTES)) else {
            continue;
        };
        let md = html_to_markdown(&decode_bytes(&buf));
        if md.trim().is_empty() {
            continue;
        }
        total += md.len();
        parts.push(md);
    }
    if parts.is_empty() {
        return Err(anyhow::anyhow!("This EPUB contains no readable text"));
    }
    Ok(pages_to_markdown(&[parts.join("\n\n")]).markdown)
}

/// YAML-ish front matter for a document's Markdown file. Values are quoted
/// and escaped so arbitrary titles can't break the block.
pub fn front_matter(fields: &[(&str, Option<String>)]) -> String {
    let esc = |v: &str| {
        v.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    };
    let mut s = String::from("---\n");
    for (k, v) in fields {
        if let Some(v) = v.as_ref().filter(|v| !v.trim().is_empty()) {
            s.push_str(&format!("{k}: \"{}\"\n", esc(v.trim())));
        }
    }
    s.push_str("---\n\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn truncate_text_bounds_on_char_boundary() {
        assert_eq!(truncate_text("hello".to_string()), "hello");
        let big = "é".repeat(MAX_TEXT_BYTES as usize);
        let out = truncate_text(big);
        assert!(out.len() <= MAX_TEXT_BYTES as usize);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn decodes_utf16_and_latin1() {
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("hé".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        assert_eq!(decode_bytes(&utf16), "hé");
        let latin1 = b"caf\xe9 au lait, tr\xe8s bon, d\xe9j\xe0 vu";
        assert!(decode_bytes(latin1).starts_with("café"));
    }

    #[test]
    fn spine_order_skips_nav_and_resolves_dir() {
        let opf = r#"<package><manifest>
            <item id="nav" href="nav.xhtml" properties="nav"/>
            <item id="c2" href="ch2.xhtml"/>
            <item id="c1" href="ch%201.xhtml"/>
            </manifest><spine><itemref idref="nav"/><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
        assert_eq!(
            spine_paths(opf, "OEBPS"),
            vec!["OEBPS/ch 1.xhtml", "OEBPS/ch2.xhtml"]
        );
    }

    #[test]
    fn epub_extracts_in_spine_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.epub");
        let f = std::fs::File::create(&p).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(
            br#"<container><rootfiles><rootfile full-path="OPS/c.opf"/></rootfiles></container>"#,
        )
        .unwrap();
        z.start_file("OPS/c.opf", o).unwrap();
        z.write_all(br#"<package><manifest><item id="a" href="a.xhtml"/><item id="b" href="b.xhtml"/></manifest><spine><itemref idref="b"/><itemref idref="a"/></spine></package>"#).unwrap();
        z.start_file("OPS/a.xhtml", o).unwrap();
        z.write_all(b"<html><body><p>Second chapter text.</p></body></html>")
            .unwrap();
        z.start_file("OPS/b.xhtml", o).unwrap();
        z.write_all(b"<html><body><p>First chapter text.</p></body></html>")
            .unwrap();
        z.finish().unwrap();
        let out = extract(&p, "Book").unwrap();
        let first = out.markdown.find("First chapter").unwrap();
        let second = out.markdown.find("Second chapter").unwrap();
        assert!(first < second, "{}", out.markdown);
        assert!(!out.chunks.is_empty());
    }

    #[test]
    fn html_document_extracts_main_text() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.html");
        let body = "Rivers carve valleys over thousands of years, carrying sediment from the mountains down to the sea. \
            Engineers study how floods reshape the banks and how dams change the flow of water through each season. \
            Communities along the river depend on it for drinking water, farming, transport and fishing. ".repeat(3);
        std::fs::write(&p, format!("<html><body><nav>menu</nav><article><h1>Rivers</h1><p>{body}</p></article></body></html>")).unwrap();
        let out = extract(&p, "Rivers").unwrap();
        assert!(out.markdown.contains("## Rivers"));
        assert!(!out.markdown.contains("menu"));
        assert_eq!(out.lang.as_deref(), Some("eng"));
        assert!(out.words > 100);
    }

    #[test]
    fn front_matter_escapes_values() {
        let fm = front_matter(&[("title", Some("A \"quoted\" title".into())), ("doi", None)]);
        assert_eq!(fm, "---\ntitle: \"A \\\"quoted\\\" title\"\n---\n\n");
    }

    #[test]
    fn empty_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.txt");
        std::fs::write(&p, "   ").unwrap();
        assert!(extract(&p, "e").is_err());
    }
}
