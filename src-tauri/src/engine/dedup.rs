//! Cross-source deduplication for discovered documents.
//!
//! The same paper commonly surfaces from arXiv, Semantic Scholar, OpenAlex,
//! and a handful of web search engines. URL-only dedup (the previous
//! behavior) misses these because each source links to its own canonical URL.
//! Three fingerprints are checked in priority order:
//!
//!   1. DOI — extracted from the URL or `identifier` field. Authoritative.
//!   2. Normalized title — lowercased, stripped of punctuation and HTML
//!      entities, collapsed to ASCII. Catches papers without DOIs.
//!   3. (first author lastname, year, first 5 title words) fingerprint.
//!      Backstop for papers with neither DOI nor an exact title match.
//!
//! When a duplicate is found the `MergedDoc` accumulates additional
//! `(source, rank_in_source)` entries — both for showing multi-source
//! attribution in the UI and for Reciprocal Rank Fusion downstream.

use crate::sources::Document;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

static DOI_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\b(10\.\d{4,9}/[-._;()/:A-Z0-9]+)"#).unwrap());
static NON_ALNUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\w\s]").unwrap());
static WHITESPACE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// One discovered document plus all sources that returned it and the rank
/// it held within each source's result list. Ranks are 1-indexed.
#[derive(Debug, Clone)]
pub struct MergedDoc {
    pub doc: Document,
    /// `(source_id, rank_in_source)` — first entry is the original discovery.
    pub source_ranks: Vec<(String, usize)>,
}

impl MergedDoc {
    pub fn sources(&self) -> Vec<String> {
        self.source_ranks.iter().map(|(s, _)| s.clone()).collect()
    }
}

/// Strips a DOI from a URL string or returns None if no DOI is present.
/// Strips common journal-resource suffixes that DOIs sometimes accumulate
/// when embedded in URLs (e.g. `.full`, `.pdf`, `.html`).
pub fn extract_doi(s: &str) -> Option<String> {
    let raw = DOI_RE.find(s)?.as_str().to_lowercase();
    let trimmed = raw.trim_end_matches(['.', ',', ')', ';']);
    let stripped = trimmed
        .trim_end_matches(".full")
        .trim_end_matches(".abstract")
        .trim_end_matches(".pdf")
        .trim_end_matches(".html")
        .trim_end_matches(".xml")
        .trim_end_matches('.');
    Some(stripped.to_string())
}

/// Title normalization for fuzzy matching. Lowercases, strips punctuation,
/// and collapses whitespace. Truncated to 200 chars so trivially different
/// suffixes still collide ("Foo: a study" vs "Foo: a study (revised)") without
/// false-merging genuinely distinct long titles that merely share a leading
/// phrase (common in survey/review papers, which routinely exceed 80 chars).
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let no_punct = NON_ALNUM_RE.replace_all(&lower, " ");
    let collapsed = WHITESPACE_RE.replace_all(&no_punct, " ");
    let trimmed: String = collapsed.trim().chars().take(200).collect();
    trimmed
}

/// Guard for the author/year backstop (step 4 of `add`): two papers sharing only
/// a common first author, year, and generic 3-word title prefix ("the role of
/// …") must NOT be merged. Require the titles to actually corroborate — one
/// normalized title being a prefix of the other (the reissue / "Revised Edition"
/// case the backstop is meant to catch).
fn titles_corroborate(a: &str, b: &str) -> bool {
    let (na, nb) = (normalize_title(a), normalize_title(b));
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na.starts_with(&nb) || nb.starts_with(&na)
}

fn author_year_fingerprint(doc: &Document) -> Option<String> {
    let raw_first_author = doc.authors.first()?;
    // Two common formats: "Doe, Jane" (bibliographic) and "Jane Doe" (display).
    // Detect comma form; otherwise the lastname is the final whitespace-token.
    let last = if raw_first_author.contains(',') {
        raw_first_author.split(',').next()?.trim().to_string()
    } else {
        raw_first_author.split_whitespace().last()?.to_string()
    }
    .to_lowercase();
    let year = doc.year.as_deref().filter(|y| y.len() == 4)?;
    // First 3 normalized title tokens (alphanumeric only). Catches reissues
    // that append " — Revised Edition" / " (2nd ed.)" without losing precision.
    let title_part: String = doc
        .title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if title_part.is_empty() {
        return None;
    }
    Some(format!("{}|{}|{}", last, year, title_part))
}

#[derive(Debug, Default)]
pub struct Deduplicator {
    docs: Vec<MergedDoc>,
    by_doi: HashMap<String, usize>,
    by_title: HashMap<String, usize>,
    by_author_year: HashMap<String, usize>,
    by_url: HashMap<String, usize>,
}

#[derive(Debug)]
pub enum AddOutcome {
    /// Newly inserted; idx is its position in `docs`.
    New(usize),
    /// Merged into an existing doc; idx is the existing entry.
    Merged(usize),
}

impl Deduplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or merge a discovered document. The `rank` is the result's
    /// 1-indexed position within `source_id`'s result list, used for RRF.
    pub fn add(&mut self, doc: Document, source_id: &str, rank: usize) -> AddOutcome {
        // 1. URL exact match — fastest, also catches re-emissions of same source.
        if let Some(&idx) = self.by_url.get(&doc.url) {
            self.docs[idx]
                .source_ranks
                .push((source_id.to_string(), rank));
            return AddOutcome::Merged(idx);
        }

        // 2. DOI match — authoritative when present.
        let doi = extract_doi(&doc.url).or_else(|| doc.identifier.as_deref().and_then(extract_doi));
        if let Some(ref d) = doi {
            if let Some(&idx) = self.by_doi.get(d) {
                self.docs[idx]
                    .source_ranks
                    .push((source_id.to_string(), rank));
                self.by_url.insert(doc.url.clone(), idx);
                return AddOutcome::Merged(idx);
            }
        }

        // 3. Normalized-title match.
        let norm = normalize_title(&doc.title);
        if !norm.is_empty() {
            if let Some(&idx) = self.by_title.get(&norm) {
                self.docs[idx]
                    .source_ranks
                    .push((source_id.to_string(), rank));
                self.by_url.insert(doc.url.clone(), idx);
                if let Some(d) = &doi {
                    self.by_doi.insert(d.clone(), idx);
                }
                return AddOutcome::Merged(idx);
            }
        }

        // 4. (author, year, title-prefix) fingerprint — last-resort backstop.
        //    Only accept it when the full titles corroborate, so distinct papers
        //    that merely share an author, year, and generic 3-word prefix aren't
        //    collapsed (which would silently drop the second from the results).
        let ay = author_year_fingerprint(&doc);
        if let Some(ref ay_key) = ay {
            if let Some(&idx) = self.by_author_year.get(ay_key) {
                if titles_corroborate(&doc.title, &self.docs[idx].doc.title) {
                    self.docs[idx]
                        .source_ranks
                        .push((source_id.to_string(), rank));
                    self.by_url.insert(doc.url.clone(), idx);
                    self.by_title.insert(norm, idx);
                    return AddOutcome::Merged(idx);
                }
            }
        }

        // 5. Insert as new entry.
        let idx = self.docs.len();
        self.by_url.insert(doc.url.clone(), idx);
        if let Some(d) = doi {
            self.by_doi.insert(d, idx);
        }
        if !norm.is_empty() {
            self.by_title.insert(norm, idx);
        }
        if let Some(ay_key) = ay {
            // Keep the FIRST doc as the fingerprint anchor; a later doc that
            // shared the fingerprint but failed title corroboration must not
            // hijack the bucket away from the original.
            self.by_author_year.entry(ay_key).or_insert(idx);
        }
        self.docs.push(MergedDoc {
            doc,
            source_ranks: vec![(source_id.to_string(), rank)],
        });
        AddOutcome::New(idx)
    }

    pub fn into_docs(self) -> Vec<MergedDoc> {
        self.docs
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(title: &str, url: &str, authors: &[&str], year: Option<&str>) -> Document {
        Document {
            title: title.to_string(),
            url: url.to_string(),
            source: "test".into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            year: year.map(String::from),
            abstract_: None,
            identifier: None,
        }
    }

    #[test]
    fn extracts_doi_from_url() {
        assert_eq!(
            extract_doi("https://doi.org/10.1038/nature12373.full"),
            Some("10.1038/nature12373".to_string())
        );
        assert_eq!(extract_doi("https://example.com/article.html"), None);
    }

    #[test]
    fn normalize_title_collapses_punctuation() {
        let a = normalize_title("Attention Is All You Need!");
        let b = normalize_title("attention is all you need");
        assert_eq!(a, b);
    }

    #[test]
    fn doi_dedup_merges_sources() {
        let mut d = Deduplicator::new();
        let r1 = d.add(
            doc(
                "X",
                "https://arxiv.org/abs/10.1038/foo",
                &["A"],
                Some("2024"),
            ),
            "arxiv",
            1,
        );
        let r2 = d.add(
            doc(
                "X",
                "https://semscholar.org/paper/10.1038/foo.pdf",
                &["A"],
                Some("2024"),
            ),
            "semantic_scholar",
            3,
        );
        assert!(matches!(r1, AddOutcome::New(0)));
        assert!(matches!(r2, AddOutcome::Merged(0)));
        let docs = d.into_docs();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].source_ranks.len(), 2);
        assert_eq!(docs[0].source_ranks[0].0, "arxiv");
        assert_eq!(docs[0].source_ranks[1].0, "semantic_scholar");
    }

    #[test]
    fn title_dedup_merges_no_doi() {
        let mut d = Deduplicator::new();
        let _ = d.add(
            doc("Attention Is All You Need", "https://a.com/p1", &[], None),
            "web",
            1,
        );
        let r = d.add(
            doc("Attention is all you need.", "https://b.com/p2", &[], None),
            "brave",
            2,
        );
        assert!(matches!(r, AddOutcome::Merged(0)));
    }

    #[test]
    fn url_dedup_within_source() {
        let mut d = Deduplicator::new();
        let _ = d.add(doc("X", "https://a/p", &[], None), "web", 1);
        let r = d.add(doc("X", "https://a/p", &[], None), "web", 1);
        assert!(matches!(r, AddOutcome::Merged(0)));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn author_year_fallback() {
        let mut d = Deduplicator::new();
        let _ = d.add(
            doc(
                "Different Title String",
                "https://a/p1",
                &["Smith, John"],
                Some("2023"),
            ),
            "web",
            1,
        );
        // Same author+year+title-prefix but different title suffix and URL.
        let r = d.add(
            doc(
                "Different Title String — Revised Edition",
                "https://b/p2",
                &["John Smith"],
                Some("2023"),
            ),
            "brave",
            1,
        );
        assert!(matches!(r, AddOutcome::Merged(0)));
    }
}
