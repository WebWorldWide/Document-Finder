//! Utilities for natural language query parsing, sub-query expansion, and relevance scoring.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

static STOPWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "please",
        "find",
        "all",
        "the",
        "a",
        "an",
        "to",
        "for",
        "of",
        "and",
        "or",
        "in",
        "on",
        "with",
        "about",
        "relating",
        "related",
        "documents",
        "document",
        "pdfs",
        "pdf",
        "books",
        "book",
        "papers",
        "paper",
        "texts",
        "text",
        "be",
        "how",
        "can",
        "you",
        "me",
        "my",
        "i",
        "are",
        "is",
        "scholarly",
        "academic",
        "articles",
        "article",
        "any",
        "some",
        "that",
        "which",
        "this",
        "these",
        "those",
        "etc",
        "every",
        "kind",
        "kinds",
    ]
    .into_iter()
    .collect()
});

static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:[A-Za-z][A-Za-z'-]+|\b\d{4}\b)").unwrap());
static SPLIT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)[,;]|\s+and\s+|\s+&\s+").unwrap());

/// Strip filler words from a natural-language query.
pub fn parse_query(q: &str) -> Vec<String> {
    let lower = q.to_lowercase();
    WORD_RE
        .find_iter(&lower)
        .map(|m| m.as_str().to_string())
        .filter(|w| !STOPWORDS.contains(w.as_str()) && w.chars().count() > 2)
        .collect()
}

/// Split a multi-topic query into independently searchable sub-queries.
///
/// ```text
/// expand_query("Christian bibles, scholarly texts, and patristic writings")
///   -> ["Christian bibles", "scholarly texts", "patristic writings"]
/// ```
pub fn expand_query(q: &str) -> Vec<String> {
    let parts: Vec<String> = SPLIT_RE
        .split(q)
        .map(|s| s.trim_matches(|c: char| c == ' ' || c == '.').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        vec![q.to_string()]
    } else {
        parts
    }
}

/// Number of distinct keywords that appear (case-insensitive substring match)
/// in `text`. Used as a relevance signal when filtering candidates before
/// download — a paper that doesn't mention any of the query's load-bearing
/// words in its title or abstract is almost certainly off-topic.
pub fn relevance_score(keywords: &[String], text: &str) -> usize {
    if keywords.is_empty() {
        return 0;
    }
    let lower = text.to_lowercase();
    keywords
        .iter()
        .filter(|k| {
            let kl = k.to_lowercase();
            !kl.is_empty() && lower.contains(&kl)
        })
        .count()
}

/// Folder-safe slug derived from a query string.
/// A short Unix timestamp suffix is appended so concurrent or repeated runs
/// with similar queries never collide on the same output directory.
pub fn safe_folder(query: &str) -> String {
    static NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\w\s-]").unwrap());
    static HYPHEN_RUNS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[-\s]+").unwrap());
    let cleaned = NON_ALNUM.replace_all(query, "");
    let trimmed = cleaned.trim().to_lowercase();
    let with_hyphens = HYPHEN_RUNS.replace_all(&trimmed, "-");
    let slug: String = with_hyphens.chars().take(48).collect();
    let base = if slug.is_empty() { "library".to_string() } else { slug };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}-{}", base, ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_canonical() {
        let r = expand_query("Christian bibles, scholarly texts, and patristic writings");
        assert_eq!(
            r,
            vec!["Christian bibles", "scholarly texts", "patristic writings"]
        );
    }

    #[test]
    fn expand_ampersand() {
        let r = expand_query("foo & bar");
        assert_eq!(r, vec!["foo", "bar"]);
    }

    #[test]
    fn parse_drops_stopwords_and_short() {
        let kw = parse_query("Please find all the books on therapy training.");
        assert_eq!(kw, vec!["therapy", "training"]);
    }

    #[test]
    fn safe_folder_basic() {
        let slug = safe_folder("Christian bibles, scholarly texts");
        assert!(
            slug.starts_with("christian-bibles-scholarly-texts-"),
            "unexpected slug: {}",
            slug
        );
        // Timestamp suffix is at least 10 digits (Unix seconds since 2001).
        let suffix = slug.rsplit_once('-').unwrap().1;
        assert!(suffix.parse::<u64>().is_ok(), "non-numeric suffix: {}", suffix);
    }

    #[test]
    fn relevance_zero_when_no_match() {
        let kw = vec!["christian".to_string(), "patristic".to_string()];
        assert_eq!(relevance_score(&kw, "Effect of Inquiry Learning Model"), 0);
    }

    #[test]
    fn relevance_counts_distinct_matches() {
        let kw = vec!["christian".to_string(), "bibles".to_string()];
        assert_eq!(
            relevance_score(&kw, "A history of Christian bibles in Africa"),
            2
        );
    }

    #[test]
    fn relevance_is_case_insensitive() {
        let kw = vec!["CHRISTIAN".to_string()];
        assert_eq!(relevance_score(&kw, "christian migration patterns"), 1);
    }

    #[test]
    fn relevance_handles_substrings() {
        let kw = vec!["christian".to_string()];
        // "Christianization" contains "christian" — counted as match.
        assert_eq!(relevance_score(&kw, "Christianization in early Europe"), 1);
    }

    #[test]
    fn relevance_empty_keywords() {
        let kw: Vec<String> = vec![];
        assert_eq!(relevance_score(&kw, "anything"), 0);
    }
}
