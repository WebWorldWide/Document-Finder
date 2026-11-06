//! Read-side access to saved libraries: document listings, full-text search
//! over chunks, context-budgeted corpora for AI, and the AI export pack.
//! Shared by the Tauri commands and the MCP server.

use super::db::{fts_query, open_read_only, DbManager, SCHEMA_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Open a library DB read-only, upgrading a 0.2-era schema first if needed.
pub fn open_library_db(folder: &Path) -> Result<Connection, String> {
    let db = folder.join("library.db");
    if !db.exists() {
        return Err("library.db not found".into());
    }
    let version: i64 = open_read_only(&db)
        .and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
        .unwrap_or(0);
    if version < SCHEMA_VERSION {
        DbManager::new(&db).map_err(|e| format!("Couldn't upgrade library: {e}"))?;
    }
    open_read_only(&db).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct LibrarySummary {
    pub name: String,
    pub path: String,
    pub query: String,
    pub n_docs: usize,
}

/// Every library folder under `root` (non-recursive), newest first.
pub fn list_libraries(root: &Path) -> Vec<LibrarySummary> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() || !p.join("library.db").exists() {
            continue;
        }
        let Ok(conn) = open_library_db(&p) else {
            continue;
        };
        let row = conn
            .query_row(
                "SELECT r.query, (SELECT COUNT(*) FROM documents) FROM runs r
                 ORDER BY r.created_at DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        if let Some((query, n_docs)) = row {
            out.push(LibrarySummary {
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: p.to_string_lossy().to_string(),
                query,
                n_docs,
            });
        }
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DocInfo {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub source: String,
    pub sources: Vec<String>,
    pub authors: Vec<String>,
    pub year: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_: Option<String>,
    pub doi: Option<String>,
    pub venue: Option<String>,
    pub license: Option<String>,
    pub cited_by: Option<i64>,
    pub score: Option<f64>,
    /// Absolute path to the original file.
    pub path: String,
    /// Absolute path to the extracted Markdown, if any.
    pub md_path: Option<String>,
    pub size_bytes: Option<u64>,
    pub extract_error: Option<String>,
    pub pages: Option<i64>,
    pub lang: Option<String>,
    pub words: Option<i64>,
    pub tokens: Option<i64>,
    pub retrieved_at: Option<String>,
}

fn split_list(s: Option<String>, sep: char) -> Vec<String> {
    s.map(|v| {
        v.split(sep)
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

/// Resolve a stored relative path inside `folder`, refusing anything that
/// escapes it (legacy rows may hold absolute paths or `..`).
fn contained(folder: &Path, rel: &str) -> Option<PathBuf> {
    let canon_folder = folder.canonicalize().ok()?;
    let joined = folder.join(rel);
    let canon = joined.canonicalize().ok()?;
    canon.starts_with(&canon_folder).then_some(joined)
}

const DOC_COLUMNS: &str = "id, title, url, source, sources, authors, year, abstract, doi, venue,
    license, cited_by, score, local_path, COALESCE(md_path, text_path), size_bytes, extract_error,
    pages, lang, words, tokens, retrieved_at";

fn doc_from_row(folder: &Path, r: &rusqlite::Row<'_>) -> rusqlite::Result<Option<DocInfo>> {
    let local: String = r.get(13)?;
    let Some(path) = contained(folder, &local) else {
        return Ok(None);
    };
    let md: Option<String> = r.get(14)?;
    Ok(Some(DocInfo {
        id: r.get(0)?,
        title: r.get(1)?,
        url: r.get(2)?,
        source: r.get(3)?,
        sources: split_list(r.get(4)?, ','),
        authors: split_list(r.get(5)?, ','),
        year: r.get(6)?,
        abstract_: r.get(7)?,
        doi: r.get(8)?,
        venue: r.get(9)?,
        license: r.get(10)?,
        cited_by: r.get(11)?,
        score: r.get(12)?,
        path: path.to_string_lossy().to_string(),
        md_path: md
            .and_then(|m| contained(folder, &m))
            .map(|p| p.to_string_lossy().to_string()),
        size_bytes: r.get::<_, Option<i64>>(15)?.map(|n| n.max(0) as u64),
        extract_error: r.get(16)?,
        pages: r.get(17)?,
        lang: r.get(18)?,
        words: r.get(19)?,
        tokens: r.get(20)?,
        retrieved_at: r.get(21)?,
    }))
}

/// All documents in a library whose file still exists, best-scored first.
pub fn list_docs(folder: &Path) -> Result<Vec<DocInfo>, String> {
    let conn = open_library_db(folder)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {DOC_COLUMNS} FROM documents
             WHERE local_path IS NOT NULL AND local_path != ''
             ORDER BY score DESC NULLS LAST, title COLLATE NOCASE"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| doc_from_row(folder, r))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok().flatten()).collect())
}

pub fn get_doc(folder: &Path, id: i64) -> Result<Option<DocInfo>, String> {
    let conn = open_library_db(folder)?;
    conn.query_row(
        &format!("SELECT {DOC_COLUMNS} FROM documents WHERE id = ?1"),
        params![id],
        |r| doc_from_row(folder, r),
    )
    .optional()
    .map(|o| o.flatten())
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub library: String,
    pub library_query: String,
    pub doc_id: i64,
    pub chunk_id: i64,
    pub title: String,
    pub heading: Option<String>,
    pub page_start: Option<i64>,
    pub page_end: Option<i64>,
    /// Matched passage with `[` `]` around hits.
    pub snippet: String,
    pub rank: f64,
}

/// Full-text search over one library's chunks (BM25 via FTS5).
pub fn search(folder: &Path, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let Some(q) = fts_query(query) else {
        return Ok(Vec::new());
    };
    let conn = open_library_db(folder)?;
    let lib_query: String = conn
        .query_row(
            "SELECT query FROM runs ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let mut stmt = conn
        .prepare(
            "SELECT c.doc_id, c.id, d.title, c.heading, c.page_start, c.page_end,
                    snippet(chunks_fts, 0, '[', ']', '…', 28), bm25(chunks_fts, 1.0, 2.0)
             FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid
             JOIN documents d ON d.id = c.doc_id
             WHERE chunks_fts MATCH ?1
             ORDER BY bm25(chunks_fts, 1.0, 2.0) LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let lib = folder.to_string_lossy().to_string();
    let rows = stmt
        .query_map(params![q, limit as i64], |r| {
            Ok(SearchHit {
                library: lib.clone(),
                library_query: lib_query.clone(),
                doc_id: r.get(0)?,
                chunk_id: r.get(1)?,
                title: r.get(2)?,
                heading: r.get(3)?,
                page_start: r.get(4)?,
                page_end: r.get(5)?,
                snippet: r.get(6)?,
                rank: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Search every library under `root`, merged by BM25 rank.
pub fn search_all(root: &Path, query: &str, limit: usize) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = list_libraries(root)
        .into_iter()
        .flat_map(|l| search(Path::new(&l.path), query, limit).unwrap_or_default())
        .collect();
    hits.sort_by(|a, b| a.rank.total_cmp(&b.rank));
    hits.truncate(limit);
    hits
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkText {
    pub ord: i64,
    pub heading: Option<String>,
    pub page_start: Option<i64>,
    pub page_end: Option<i64>,
    pub text: String,
}

pub fn chunks(folder: &Path, doc_id: i64, from: i64, count: i64) -> Result<Vec<ChunkText>, String> {
    let conn = open_library_db(folder)?;
    let mut stmt = conn
        .prepare(
            "SELECT ord, heading, page_start, page_end, text FROM chunks
             WHERE doc_id = ?1 AND ord >= ?2 ORDER BY ord LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![doc_id, from, count], |r| {
            Ok(ChunkText {
                ord: r.get(0)?,
                heading: r.get(1)?,
                page_start: r.get(2)?,
                page_end: r.get(3)?,
                text: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Read a document's Markdown (front matter included), bounded.
pub fn read_markdown(doc: &DocInfo, max_bytes: usize) -> Option<String> {
    let p = doc.md_path.as_ref()?;
    let mut s = std::fs::read_to_string(p).ok()?;
    if s.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n\n[… truncated]\n");
    }
    Some(s)
}

/// A one-line citation, e.g. `Doe, Roe (2021). Title. Venue. doi:10.x/y`.
pub fn citation(d: &DocInfo) -> String {
    let mut s = String::new();
    if !d.authors.is_empty() {
        let names: Vec<&str> = d.authors.iter().take(3).map(String::as_str).collect();
        s.push_str(&names.join(", "));
        if d.authors.len() > 3 {
            s.push_str(" et al.");
        }
        s.push(' ');
    }
    if let Some(y) = &d.year {
        s.push_str(&format!("({y}). "));
    }
    s.push_str(d.title.trim().trim_end_matches('.'));
    s.push('.');
    if let Some(v) = &d.venue {
        s.push_str(&format!(" {v}."));
    }
    if let Some(doi) = &d.doi {
        s.push_str(&format!(" doi:{doi}"));
    } else {
        s.push_str(&format!(" {}", d.url));
    }
    s
}

fn bib_key(d: &DocInfo, used: &mut std::collections::HashSet<String>) -> String {
    let surname = d
        .authors
        .first()
        .map(|a| {
            if a.contains(',') {
                a.split(',').next().unwrap_or(a).to_string()
            } else {
                a.split_whitespace().last().unwrap_or(a).to_string()
            }
        })
        .unwrap_or_else(|| "anon".into());
    let word = d
        .title
        .split_whitespace()
        .find(|w| w.len() > 3)
        .unwrap_or("doc");
    let base: String = format!("{}{}{}", surname, d.year.clone().unwrap_or_default(), word)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let mut key = base.clone();
    let mut n = 2;
    while !used.insert(key.clone()) {
        key = format!("{base}{n}");
        n += 1;
    }
    key
}

fn bib_escape(s: &str) -> String {
    s.replace(['{', '}'], "").replace('\n', " ")
}

pub fn bibtex(docs: &[DocInfo]) -> String {
    let mut used = std::collections::HashSet::new();
    let mut out = String::new();
    for d in docs {
        let key = bib_key(d, &mut used);
        out.push_str(&format!(
            "@misc{{{key},\n  title = {{{}}},\n",
            bib_escape(&d.title)
        ));
        if !d.authors.is_empty() {
            out.push_str(&format!(
                "  author = {{{}}},\n",
                bib_escape(&d.authors.join(" and "))
            ));
        }
        if let Some(y) = &d.year {
            out.push_str(&format!("  year = {{{}}},\n", bib_escape(y)));
        }
        if let Some(v) = &d.venue {
            out.push_str(&format!("  howpublished = {{{}}},\n", bib_escape(v)));
        }
        if let Some(doi) = &d.doi {
            out.push_str(&format!("  doi = {{{}}},\n", bib_escape(doi)));
        }
        out.push_str(&format!("  url = {{{}}}\n}}\n\n", bib_escape(&d.url)));
    }
    out
}

fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut out = String::new();
    for part in cleaned.split('-').filter(|p| !p.is_empty()).take(10) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(part);
    }
    out.chars().take(70).collect::<String>().to_lowercase()
}

/// A single Markdown document that packs as many documents as fit in
/// `budget_tokens`, best-scored first. Documents that don't fit whole are
/// represented by their abstract and first chunks, so the AI still sees
/// every source.
pub fn build_corpus(query: &str, docs: &[DocInfo], budget_tokens: usize) -> String {
    let mut out = format!(
        "# Research corpus: {query}\n\nCompiled by Document Finder on {}. {} documents. \
         Each section starts with its citation; cite sources by their number.\n\n",
        chrono::Utc::now().format("%Y-%m-%d"),
        docs.len()
    );
    let mut used = super::textclean::estimate_tokens(&out);
    let mut sections = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let header = format!(
            "\n---\n\n## [{}] {}\n\n*{}*\n\n",
            i + 1,
            d.title.trim(),
            citation(d)
        );
        let body = read_markdown(d, 4 << 20)
            .map(|md| strip_front_matter(&md).to_string())
            .unwrap_or_else(|| d.abstract_.clone().unwrap_or_default());
        let need =
            super::textclean::estimate_tokens(&header) + super::textclean::estimate_tokens(&body);
        let remaining = budget_tokens.saturating_sub(used);
        if need <= remaining {
            used += need;
            sections.push(header + &body);
        } else if remaining > 300 {
            // Partial: as much of the body as fits.
            let take_chars = (remaining - 120) * 4;
            let mut cut: String = body.chars().take(take_chars).collect();
            cut.push_str("\n\n[… truncated to fit the context budget]\n");
            used = budget_tokens;
            sections.push(header + &cut);
        } else {
            break;
        }
    }
    out.push_str(&sections.join("\n"));
    out
}

/// One self-contained Markdown file for RAG ingestion: every document's full
/// clean text, no context budget. Each document is a top-level `#` section
/// (so heading-aware splitters keep documents apart) followed by a metadata
/// list a retriever can surface as the citation; the document's own section
/// headings sit below it (`##`+). Page markers (`<!-- page N -->`) are kept so
/// chunks can cite pages.
pub fn rag_markdown(query: &str, docs: &[DocInfo]) -> String {
    let mut out = format!(
        "---\ntitle: \"Research library: {}\"\ngenerator: \"Document Finder\"\ncreated: \"{}\"\ndocuments: {}\n---\n\n",
        query.replace('"', "'"),
        chrono::Utc::now().format("%Y-%m-%d"),
        docs.len()
    );
    out.push_str("<!-- Each document below starts with a level-1 heading and a metadata list. Cite sources by their citation line. -->\n");
    for (i, d) in docs.iter().enumerate() {
        out.push_str(&format!("\n# [{}] {}\n\n", i + 1, d.title.trim()));
        out.push_str(&format!("- **Citation:** {}\n", citation(d)));
        if !d.authors.is_empty() {
            out.push_str(&format!("- **Authors:** {}\n", d.authors.join("; ")));
        }
        if let Some(y) = &d.year {
            out.push_str(&format!("- **Year:** {y}\n"));
        }
        if let Some(doi) = &d.doi {
            out.push_str(&format!("- **DOI:** {doi}\n"));
        }
        out.push_str(&format!("- **URL:** {}\n", d.url));
        out.push_str(&format!(
            "- **Source:** {}\n",
            d.sources.join(", ").if_empty(&d.source)
        ));
        if let Some(l) = &d.license {
            out.push_str(&format!("- **License:** {l}\n"));
        }
        out.push('\n');
        let body = read_markdown(d, 32 << 20)
            .map(|md| {
                let b = strip_front_matter(&md);
                // Drop the document's own `# Title` line (already the heading above).
                match b.strip_prefix("# ") {
                    Some(rest) => rest
                        .split_once('\n')
                        .map(|(_, r)| r.to_string())
                        .unwrap_or_default(),
                    None => b.to_string(),
                }
            })
            .or_else(|| {
                d.abstract_
                    .clone()
                    .map(|a| format!("> **Abstract.** {a}\n"))
            })
            .unwrap_or_else(|| {
                format!(
                    "*No extractable text{}.*\n",
                    d.extract_error
                        .as_ref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                )
            });
        out.push_str(body.trim());
        out.push('\n');
    }
    out
}

trait IfEmpty {
    fn if_empty(self, other: &str) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, other: &str) -> String {
        if self.is_empty() {
            other.to_string()
        } else {
            self
        }
    }
}

pub fn strip_front_matter(md: &str) -> &str {
    if let Some(rest) = md.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            return rest[end + 5..].trim_start();
        }
    }
    md
}

#[derive(Debug, Serialize)]
pub struct PackResult {
    pub dest: String,
    pub files: usize,
    pub size_bytes: u64,
}

/// Write the AI export pack as a zip:
///
/// ```text
/// <library>/README.md        index: query, one line per document, how to use
/// <library>/documents/*.md   clean Markdown with front matter
/// <library>/chunks.jsonl     retrieval-ready chunks with citations
/// <library>/references.bib   BibTeX for every document
/// <library>/corpus.md        everything that fits `budget_tokens` (if set)
/// <library>/originals/…      original PDFs/EPUBs (if `include_originals`)
/// ```
pub fn write_ai_pack(
    folder: &Path,
    dest: &Path,
    include_originals: bool,
    budget_tokens: Option<usize>,
) -> Result<PackResult, String> {
    let docs = list_docs(folder)?;
    let conn = open_library_db(folder)?;
    let query: String = conn
        .query_row(
            "SELECT query FROM runs ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let root_name = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "library".into());
    let tmp = dest.with_extension("zip.partial");
    let build = || -> Result<usize, String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));
        let stored: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let n = std::cell::Cell::new(0usize);
        let put = |zw: &mut zip::ZipWriter<std::fs::File>,
                   name: &str,
                   data: &[u8]|
         -> Result<(), String> {
            zw.start_file(format!("{root_name}/{name}"), opts)
                .map_err(|e| e.to_string())?;
            zw.write_all(data).map_err(|e| e.to_string())?;
            n.set(n.get() + 1);
            Ok(())
        };

        let mut index = format!(
            "# {query}\n\nA research library compiled by [Document Finder](https://documentfinder.webworldwide.online) \
             on {}. {} documents.\n\n## How to use this with an AI\n\n\
             - Upload `corpus.md` (if present) for a single-file overview that fits a context window.\n\
             - Or upload the files in `documents/` — each is clean Markdown with a metadata header.\n\
             - `chunks.jsonl` holds ~800-token passages with citations for retrieval (RAG) pipelines.\n\
             - `references.bib` has a BibTeX entry for every document.\n\n## Documents\n\n",
            chrono::Utc::now().format("%Y-%m-%d"),
            docs.len()
        );
        let mut jsonl = String::new();
        let mut names = std::collections::HashSet::new();
        for (i, d) in docs.iter().enumerate() {
            let mut base = format!("{:03}-{}", i + 1, safe_name(&d.title));
            while !names.insert(base.clone()) {
                base.push('x');
            }
            match read_markdown(d, 32 << 20) {
                Some(md) => {
                    put(&mut zw, &format!("documents/{base}.md"), md.as_bytes())?;
                    index.push_str(&format!(
                        "{}. [{}](documents/{base}.md) — {}\n",
                        i + 1,
                        d.title.trim(),
                        citation(d)
                    ));
                }
                None => index.push_str(&format!(
                    "{}. {} — {} *(no extractable text{})*\n",
                    i + 1,
                    d.title.trim(),
                    citation(d),
                    d.extract_error
                        .as_ref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                )),
            }
            for c in chunks(folder, d.id, 0, i64::MAX).unwrap_or_default() {
                let line = serde_json::json!({
                    "id": format!("{base}#{}", c.ord),
                    "doc": base,
                    "title": d.title,
                    "citation": citation(d),
                    "url": d.url,
                    "doi": d.doi,
                    "year": d.year,
                    "section": c.heading,
                    "page_start": c.page_start,
                    "page_end": c.page_end,
                    "text": c.text,
                });
                jsonl.push_str(&line.to_string());
                jsonl.push('\n');
            }
            if include_originals {
                let src = Path::new(&d.path);
                if let (Ok(meta), Some(fname)) = (std::fs::symlink_metadata(src), src.file_name()) {
                    if meta.is_file() {
                        zw.start_file(
                            format!("{root_name}/originals/{}", fname.to_string_lossy()),
                            stored,
                        )
                        .map_err(|e| e.to_string())?;
                        let mut f = std::fs::File::open(src).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, &mut zw).map_err(|e| e.to_string())?;
                        n.set(n.get() + 1);
                    }
                }
            }
        }
        put(&mut zw, "README.md", index.as_bytes())?;
        put(&mut zw, "chunks.jsonl", jsonl.as_bytes())?;
        put(&mut zw, "references.bib", bibtex(&docs).as_bytes())?;
        if let Some(b) = budget_tokens {
            put(
                &mut zw,
                "corpus.md",
                build_corpus(&query, &docs, b).as_bytes(),
            )?;
        }
        zw.finish().map_err(|e| e.to_string())?;
        Ok(n.get())
    };
    let files = match build() {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Couldn't finish the export: {e}")
    })?;
    Ok(PackResult {
        dest: dest.to_string_lossy().to_string(),
        files,
        size_bytes: std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::{ChunkRecord, DocRecord};

    fn make_library(dir: &Path) -> PathBuf {
        let lib = dir.join("solar-panels-1");
        std::fs::create_dir_all(lib.join("_text")).unwrap();
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(5000, b' ');
        std::fs::write(lib.join("a.pdf"), &pdf).unwrap();
        std::fs::write(
            lib.join("_text/a.md"),
            "---\ntitle: \"Solar A\"\n---\n\n# Solar A\n\nPerovskite cells are efficient.\n",
        )
        .unwrap();
        let mut mgr = DbManager::new(&lib.join("library.db")).unwrap();
        let run = mgr.insert_run("solar panels", "x").unwrap();
        mgr.upsert_document(
            run,
            &DocRecord {
                title: "Solar A".into(),
                url: "https://x/a".into(),
                source: "arxiv".into(),
                authors: vec!["Jane Doe".into()],
                year: Some("2021".into()),
                doi: Some("10.1/abc".into()),
                local_path: "a.pdf".into(),
                md_path: Some("_text/a.md".into()),
                score: Some(2.0),
                ..Default::default()
            },
            &[ChunkRecord {
                ord: 0,
                heading: Some("Solar A".into()),
                text: "Perovskite cells are efficient.".into(),
                tokens: 6,
                ..Default::default()
            }],
        )
        .unwrap();
        lib
    }

    #[test]
    fn lists_searches_and_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = make_library(tmp.path());
        let libs = list_libraries(tmp.path());
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].query, "solar panels");
        let docs = list_docs(&lib).unwrap();
        assert_eq!(docs.len(), 1);
        assert!(docs[0].md_path.is_some());
        let hits = search_all(tmp.path(), "perovskite", 5);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("[Perovskite]"));
        assert_eq!(citation(&docs[0]), "Jane Doe (2021). Solar A. doi:10.1/abc");
        let corpus = build_corpus("solar panels", &docs, 10_000);
        assert!(corpus.contains("## [1] Solar A"));
        assert!(corpus.contains("Perovskite"));
        assert!(
            !corpus.contains("title: \"Solar A\""),
            "front matter stripped"
        );
    }

    #[test]
    fn ai_pack_contains_expected_files() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = make_library(tmp.path());
        let dest = tmp.path().join("out.zip");
        let r = write_ai_pack(&lib, &dest, true, Some(50_000)).unwrap();
        assert!(r.files >= 5);
        let f = std::fs::File::open(&dest).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect();
        for want in [
            "README.md",
            "chunks.jsonl",
            "references.bib",
            "corpus.md",
            "originals/a.pdf",
        ] {
            assert!(
                names.iter().any(|n| n.ends_with(want)),
                "missing {want} in {names:?}"
            );
        }
        assert!(names.iter().all(|n| !n.contains("library.db")));
    }

    #[test]
    fn rag_markdown_has_one_section_per_document() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = make_library(tmp.path());
        let docs = list_docs(&lib).unwrap();
        let md = rag_markdown("solar panels", &docs);
        assert!(md.starts_with("---\ntitle: \"Research library: solar panels\""));
        assert_eq!(md.matches("\n# [").count(), 1);
        assert!(md.contains("- **DOI:** 10.1/abc"));
        assert!(md.contains("Perovskite cells are efficient."));
        assert!(
            !md.contains("# Solar A\n\nPerovskite"),
            "own title line removed"
        );
    }

    #[test]
    fn corpus_respects_budget() {
        let d = DocInfo {
            title: "Big".into(),
            abstract_: Some("word ".repeat(10_000)),
            url: "https://x".into(),
            ..Default::default()
        };
        let c = build_corpus("q", &[d], 2_000);
        assert!(super::super::textclean::estimate_tokens(&c) <= 2_200);
        assert!(c.contains("truncated"));
    }
}
