//! Turn raw extracted page text into clean, structured Markdown and
//! retrieval-sized chunks.
//!
//! Raw PDF text is full of layout debris: running headers/footers, page
//! numbers, words hyphenated across line breaks, hard-wrapped lines,
//! ligature glyphs (`ﬁ`) and soft hyphens. This module removes that debris,
//! recovers paragraphs and section headings, and emits Markdown with page
//! markers (`<!-- page 3 -->`) so chunks can cite pages.

use crate::engine::db::ChunkRecord;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// Target chunk size in (estimated) tokens. ~800 tokens keeps a passage
/// self-contained while letting a retriever pack many into one context.
pub const CHUNK_TOKENS: usize = 800;

/// Cheap, model-free token estimate (≈ 4 chars per token for English).
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// NFKC (folds ligatures like `ﬁ` → `fi`, full-width forms, etc.), drop soft
/// hyphens / zero-width chars, normalize line endings and exotic spaces.
pub fn normalize_unicode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfkc() {
        match c {
            '\u{00AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => {}
            '\r' => {}
            '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\t' => out.push(' '),
            '\u{2010}' | '\u{2011}' => out.push('-'),
            c if c.is_control() && c != '\n' && c != '\x0c' => {}
            c => out.push(c),
        }
    }
    out
}

static PAGE_NUMBER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:page\s*)?[-–—]?\s*\d{1,4}\s*[-–—]?\s*(?:(?:of|/)\s*\d{1,4})?\s*$")
        .unwrap()
});
static ROMAN_PAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*[ivxlc]{1,6}\s*$").unwrap());
static DIGITS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d+").unwrap());
static NUMBERED_HEADING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^((?:\d{1,2}\.){0,3}\d{1,2}\.?|[IVX]{1,5}\.)\s+(\p{Lu}[\p{L}\p{N} ,:;'’()&/-]{1,80})$",
    )
    .unwrap()
});
static YEAR_PAREN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\((1[5-9]|20)\d\d[a-z]?\)").unwrap());

/// A heading never ends mid-phrase ("… of the", "… and").
fn ends_with_function_word(s: &str) -> bool {
    const W: &[&str] = &[
        "the", "of", "and", "a", "an", "to", "in", "for", "with", "on", "by", "or", "from", "at",
    ];
    s.split_whitespace()
        .last()
        .map(|w| W.contains(&w.to_lowercase().as_str()))
        .unwrap_or(false)
}

static LIST_MARK: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\(?[0-9a-z]{1,2}[.)]\s").unwrap());
static MULTISPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ ]{2,}").unwrap());

const SECTION_NAMES: &[&str] = &[
    "abstract",
    "summary",
    "introduction",
    "background",
    "related work",
    "literature review",
    "methods",
    "methodology",
    "materials and methods",
    "method",
    "experiments",
    "experimental setup",
    "results",
    "results and discussion",
    "discussion",
    "conclusion",
    "conclusions",
    "limitations",
    "future work",
    "acknowledgments",
    "acknowledgements",
    "references",
    "bibliography",
    "works cited",
    "appendix",
    "preface",
    "foreword",
    "contents",
    "table of contents",
    "index",
    "notes",
];

const REFERENCE_HEADINGS: &[&str] = &["references", "bibliography", "works cited"];

fn line_key(line: &str) -> String {
    DIGITS
        .replace_all(line.trim(), "#")
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Remove running headers/footers: lines in the first/last two positions of
/// a page whose digit-normalized form recurs on many pages. Also drops bare
/// page-number lines anywhere at a page edge.
pub fn strip_running_lines(pages: &mut [Vec<String>]) {
    let n = pages.len();
    let edge = |lines: &Vec<String>| -> Vec<usize> {
        let idx: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, _)| i)
            .collect();
        let mut out: Vec<usize> = idx.iter().take(2).copied().collect();
        for i in idx.iter().rev().take(2) {
            if !out.contains(i) {
                out.push(*i);
            }
        }
        out
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    if n >= 3 {
        for p in pages.iter() {
            let mut seen = std::collections::HashSet::new();
            for i in edge(p) {
                let k = line_key(&p[i]);
                if k.len() >= 3 && seen.insert(k.clone()) {
                    *counts.entry(k).or_insert(0) += 1;
                }
            }
        }
    }
    let threshold = ((n as f32) * 0.4).ceil().max(3.0) as usize;
    for p in pages.iter_mut() {
        let drop: Vec<usize> = edge(p)
            .into_iter()
            .filter(|&i| {
                let l = &p[i];
                PAGE_NUMBER.is_match(l)
                    || ROMAN_PAGE.is_match(l)
                    || counts.get(&line_key(l)).copied().unwrap_or(0) >= threshold
            })
            .collect();
        for i in drop {
            p[i].clear();
        }
    }
}

/// Classify a standalone line as a section heading. Returns the Markdown
/// heading level (2–4) and cleaned text.
pub fn heading_level(line: &str) -> Option<(usize, String)> {
    let t = line.trim();
    let chars = t.chars().count();
    if !(2..=90).contains(&chars) || t.ends_with(['.', ',', ';']) && !t.ends_with("..") {
        // A trailing period usually means a sentence, except "1." numbering.
        if !NUMBERED_HEADING.is_match(t) {
            return None;
        }
    }
    let lower = t.trim_end_matches(':').to_lowercase();
    if SECTION_NAMES.contains(&lower.as_str()) {
        return Some((2, title_case_if_shouty(t.trim_end_matches(':'))));
    }
    if let Some(c) = NUMBERED_HEADING.captures(t) {
        let num = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let body = c.get(2).map(|m| m.as_str()).unwrap_or("");
        let words = body.split_whitespace().count();
        let top: u32 = num
            .split('.')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        // Reject list items, wrapped sentences and reference entries that
        // merely start with a number: section numbers stay small, headings
        // are short, and citations carry a "(2001)"-style year.
        if words == 0
            || words > 10
            || body.ends_with('.')
            || (!num.contains('.') || num.matches('.').count() == 1 && num.ends_with('.'))
                && top > 15
            || YEAR_PAREN.is_match(body)
            || ends_with_function_word(body)
        {
            return None;
        }
        let depth = num.trim_end_matches('.').matches('.').count();
        return Some(((2 + depth).min(4), format!("{} {}", num, body.trim())));
    }
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    let words = t.split_whitespace().count();
    if letters.len() >= 4
        && (1..=8).contains(&words)
        && letters.iter().all(|c| c.is_uppercase())
        && !t.contains("  ")
    {
        return Some((2, title_case_if_shouty(t)));
    }
    None
}

fn title_case_if_shouty(t: &str) -> String {
    if t.chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase())
    {
        t.split_whitespace()
            .map(|w| {
                let lower = w.to_lowercase();
                let mut cs = lower.chars();
                match cs.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        t.to_string()
    }
}

fn ends_sentence(s: &str) -> bool {
    let t = s.trim_end();
    t.ends_with(['.', '?', '!', ':', '"', '”', '’', ')'])
}

fn starts_lower(s: &str) -> bool {
    s.trim_start()
        .chars()
        .next()
        .map(|c| c.is_lowercase())
        .unwrap_or(false)
}

fn is_list_item(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with(['•', '◦', '▪', '–', '-', '*']) || LIST_MARK.is_match(t)
}

/// Rebuild paragraphs from hard-wrapped lines on one page. Joins wrapped
/// lines, repairs hyphenation ("infor-\nmation" → "information"), and keeps
/// headings / list items on their own lines.
fn paragraphs_of(lines: &[String], typical_len: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let p = MULTISPACE.replace_all(cur.trim(), " ").to_string();
        if !p.is_empty() {
            out.push(p);
        }
        cur.clear();
    };
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        if let Some((lvl, text)) = heading_level(line) {
            flush(&mut cur, &mut out);
            out.push(format!("{} {}", "#".repeat(lvl), text));
            continue;
        }
        if is_list_item(line) && !cur.is_empty() {
            flush(&mut cur, &mut out);
        }
        if cur.is_empty() {
            cur.push_str(line);
        } else if cur.ends_with('-')
            && !cur.ends_with(" -")
            && !cur.ends_with("--")
            && starts_lower(line)
        {
            cur.pop();
            cur.push_str(line);
        } else {
            cur.push(' ');
            cur.push_str(line);
        }
        // A short line that ends a sentence is the end of a paragraph.
        let short = line.chars().count() < typical_len * 3 / 4;
        if short && ends_sentence(line) {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn typical_line_len(pages: &[Vec<String>]) -> usize {
    let mut lens: Vec<usize> = pages
        .iter()
        .flat_map(|p| p.iter())
        .map(|l| l.trim().chars().count())
        .filter(|&n| n >= 20)
        .collect();
    if lens.is_empty() {
        return 60;
    }
    lens.sort_unstable();
    lens[lens.len() * 3 / 4]
}

/// Structured result of cleaning a document.
pub struct Cleaned {
    pub markdown: String,
    pub words: usize,
}

/// Clean page texts (one string per page) into Markdown with page markers.
pub fn pages_to_markdown(pages: &[String]) -> Cleaned {
    let mut split: Vec<Vec<String>> = pages
        .iter()
        .map(|p| {
            normalize_unicode(p)
                .split('\n')
                .map(|l| l.trim_end().to_string())
                .collect()
        })
        .collect();
    strip_running_lines(&mut split);
    let typical = typical_line_len(&split);
    let mut md = String::new();
    let multi = split.len() > 1;
    let mut carry: Option<String> = None;
    let mut in_refs = false;
    for (i, lines) in split.iter().enumerate() {
        let mut paras = paragraphs_of(lines, typical);
        // A paragraph broken across the page boundary: merge it back.
        if let Some(prev) = carry.take() {
            match paras.first_mut() {
                Some(first) if !first.starts_with('#') && starts_lower(first) => {
                    let joined = if prev.ends_with('-') {
                        format!("{}{}", &prev[..prev.len() - 1], first)
                    } else {
                        format!("{prev} {first}")
                    };
                    *first = joined;
                }
                _ => {
                    md.push_str(&prev);
                    md.push_str("\n\n");
                }
            }
        }
        if multi {
            md.push_str(&format!("<!-- page {} -->\n\n", i + 1));
        }
        let last_open = paras
            .last()
            .map(|p| !p.starts_with('#') && !ends_sentence(p))
            .unwrap_or(false);
        if last_open && i + 1 < split.len() {
            carry = paras.pop();
        }
        for p in paras {
            // Inside a reference list, numbered entries aren't headings.
            let p = if let Some(h) = p.strip_prefix('#') {
                let text = h.trim_start_matches('#').trim();
                let lower = text.to_lowercase();
                if REFERENCE_HEADINGS.contains(&lower.as_str()) {
                    in_refs = true;
                    p
                } else if SECTION_NAMES.contains(&lower.as_str()) {
                    in_refs = false;
                    p
                } else if in_refs {
                    text.to_string()
                } else {
                    p
                }
            } else {
                p
            };
            md.push_str(&p);
            md.push_str("\n\n");
        }
    }
    if let Some(prev) = carry {
        md.push_str(&prev);
        md.push_str("\n\n");
    }
    let words = md
        .split_whitespace()
        .filter(|w| !w.starts_with("<!--") && *w != "-->" && !w.starts_with('#'))
        .count();
    Cleaned {
        markdown: md.trim_end().to_string() + "\n",
        words,
    }
}

static PAGE_MARKER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^<!-- page (\d+) -->$").unwrap());
static SENTENCE_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.!?][\s]+").unwrap());

/// Split Markdown into ~[`CHUNK_TOKENS`] chunks along paragraph boundaries,
/// tracking the current section heading and page range. The References
/// section is skipped: citation lists are noise for retrieval. `title` is
/// used as the heading of chunks before the first section.
pub fn chunk_markdown(md: &str, title: &str) -> Vec<ChunkRecord> {
    let mut chunks: Vec<ChunkRecord> = Vec::new();
    let mut heading: Option<String> = Some(title.to_string()).filter(|t| !t.is_empty());
    let mut page: Option<u32> = None;
    let mut in_refs = false;
    let mut buf: Vec<String> = Vec::new();
    let mut buf_tokens = 0usize;
    let mut start_page: Option<u32> = None;

    let emit = |buf: &mut Vec<String>,
                buf_tokens: &mut usize,
                heading: &Option<String>,
                start: Option<u32>,
                end: Option<u32>,
                chunks: &mut Vec<ChunkRecord>| {
        if buf.is_empty() {
            return;
        }
        let text = buf.join("\n\n");
        chunks.push(ChunkRecord {
            ord: chunks.len(),
            heading: heading.clone(),
            page_start: start,
            page_end: end.or(start),
            tokens: estimate_tokens(&text),
            text,
        });
        buf.clear();
        *buf_tokens = 0;
    };

    for block in md.split("\n\n") {
        let b = block.trim();
        if b.is_empty() {
            continue;
        }
        if let Some(c) = PAGE_MARKER.captures(b) {
            page = c[1].parse().ok();
            continue;
        }
        if let Some(h) = b.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim().to_string();
            emit(
                &mut buf,
                &mut buf_tokens,
                &heading,
                start_page,
                page,
                &mut chunks,
            );
            in_refs = REFERENCE_HEADINGS.contains(&h.to_lowercase().as_str());
            heading = Some(h);
            start_page = page;
            continue;
        }
        if in_refs {
            continue;
        }
        // Split an oversized paragraph on sentence boundaries.
        let pieces: Vec<String> = if estimate_tokens(b) > CHUNK_TOKENS {
            let mut v = Vec::new();
            let mut cur = String::new();
            let mut last = 0;
            for m in SENTENCE_END.find_iter(b) {
                cur.push_str(&b[last..m.end()]);
                last = m.end();
                if estimate_tokens(&cur) >= CHUNK_TOKENS / 2 {
                    v.push(cur.trim().to_string());
                    cur.clear();
                }
            }
            cur.push_str(&b[last..]);
            if !cur.trim().is_empty() {
                v.push(cur.trim().to_string());
            }
            v
        } else {
            vec![b.to_string()]
        };
        // Last resort for run-on text with no sentence breaks: hard-wrap on
        // whitespace so no chunk exceeds the budget.
        let pieces: Vec<String> = pieces
            .into_iter()
            .flat_map(|p| {
                if estimate_tokens(&p) <= CHUNK_TOKENS {
                    return vec![p];
                }
                let mut out = Vec::new();
                let mut cur = String::new();
                for w in p.split_whitespace() {
                    if estimate_tokens(&cur) + estimate_tokens(w) + 1 > CHUNK_TOKENS * 3 / 4 {
                        out.push(std::mem::take(&mut cur));
                    }
                    if !cur.is_empty() {
                        cur.push(' ');
                    }
                    cur.push_str(w);
                }
                if !cur.is_empty() {
                    out.push(cur);
                }
                out
            })
            .collect();
        for p in pieces {
            let t = estimate_tokens(&p);
            if buf_tokens + t > CHUNK_TOKENS && !buf.is_empty() {
                emit(
                    &mut buf,
                    &mut buf_tokens,
                    &heading,
                    start_page,
                    page,
                    &mut chunks,
                );
            }
            if buf.is_empty() {
                start_page = page;
            }
            buf.push(p);
            buf_tokens += t;
        }
    }
    emit(
        &mut buf,
        &mut buf_tokens,
        &heading,
        start_page,
        page,
        &mut chunks,
    );
    chunks
}

/// ISO 639-3 language code of the text, if confidently detected.
pub fn detect_language(text: &str) -> Option<String> {
    let sample: String = text.chars().take(4000).collect();
    let info = whatlang::detect(&sample)?;
    if info.is_reliable() || info.confidence() > 0.5 {
        Some(info.lang().code().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ligatures_and_soft_hyphens_are_normalized() {
        assert_eq!(
            normalize_unicode("e\u{FB03}cient\u{00AD} \u{FB01}le"),
            "efficient file"
        );
    }

    #[test]
    fn running_headers_and_page_numbers_are_removed() {
        let topics = ["rivers", "glaciers", "deserts", "forests", "oceans"];
        let pages: Vec<String> = (1..=5)
            .map(|i| {
                let t = topics[i - 1];
                format!(
                    "Journal of Things, Vol. 3, p. {i}\nThis page discusses {t} at length.\nMore detail on {t} follows here.\nAnd a closing line about {t}.\n{i}"
                )
            })
            .collect();
        let out = pages_to_markdown(&pages).markdown;
        assert!(!out.contains("Journal of Things"), "{out}");
        assert!(out.contains("This page discusses deserts"), "{out}");
        assert!(!out.lines().any(|l| l.trim() == "4"));
    }

    #[test]
    fn hyphenation_and_wrapping_are_repaired() {
        let page = "This is a long line of body text that talks about infor-\nmation retrieval and keeps\ngoing for a while.\n\nNext paragraph.".to_string();
        let out = pages_to_markdown(&[page]).markdown;
        assert!(
            out.contains("information retrieval and keeps going"),
            "{out}"
        );
        assert!(out.contains("\n\nNext paragraph."));
    }

    #[test]
    fn headings_are_detected() {
        assert_eq!(heading_level("1. Introduction").map(|h| h.0), Some(2));
        assert_eq!(heading_level("2.3 Data Collection").map(|h| h.0), Some(3));
        assert_eq!(
            heading_level("REFERENCES").map(|h| h.1),
            Some("References".into())
        );
        assert_eq!(heading_level("Abstract").map(|h| h.0), Some(2));
        assert!(heading_level("We found that 3 of the samples failed.").is_none());
        assert!(heading_level("12 apples were eaten by the committee in total.").is_none());
        assert!(
            heading_level("30. They also observed a population size-dependant fitness").is_none()
        );
        assert!(
            heading_level("3. Carroll SB (2001) Chance and necessity: the evolution of").is_none()
        );
        assert_eq!(heading_level("4 Why Self-Attention").map(|h| h.0), Some(2));
    }

    #[test]
    fn paragraphs_continue_across_pages() {
        let pages = vec![
            "Heading text\n\nThe experiment continued".to_string(),
            "into the night without stopping.".to_string(),
        ];
        let out = pages_to_markdown(&pages).markdown;
        assert!(
            out.contains("The experiment continued into the night"),
            "{out}"
        );
    }

    #[test]
    fn chunking_tracks_headings_pages_and_skips_references() {
        let body = "word ".repeat(700);
        let md = format!(
            "<!-- page 1 -->\n\n## Introduction\n\n{body}\n\n<!-- page 2 -->\n\n{body}\n\n## References\n\n[1] Some citation."
        );
        let chunks = chunk_markdown(&md, "Paper");
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].heading.as_deref(), Some("Introduction"));
        assert_eq!(chunks[0].page_start, Some(1));
        assert!(chunks.iter().all(|c| !c.text.contains("Some citation")));
        assert!(chunks.iter().all(|c| c.tokens <= CHUNK_TOKENS + 10));
        assert_eq!(chunks.last().unwrap().page_end, Some(2));
    }

    #[test]
    fn detects_english() {
        let t = "The quick brown fox jumps over the lazy dog while the researchers observe its behaviour carefully.";
        assert_eq!(detect_language(t).as_deref(), Some("eng"));
    }
}
