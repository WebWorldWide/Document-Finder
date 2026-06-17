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

static WORD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:[A-Za-z][A-Za-z'-]+|\b\d{4}\b)").unwrap());
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

/// Push `phrase` into `out` unless an equivalent search (same keyword *set*,
/// order-independent) was already added. Two phrasings that reduce to the same
/// keywords collapse to a single sub-query so the fan-out never fires redundant
/// requests at a source.
fn push_unique_subquery(
    phrase: String,
    keywords: &[String],
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if keywords.is_empty() {
        return;
    }
    let mut sig: Vec<String> = keywords.iter().map(|k| k.to_lowercase()).collect();
    sig.sort();
    let key = sig.join(" ");
    if seen.insert(key) {
        out.push(phrase);
    }
}

/// Model-free breadth expansion. Widens a query into several recall-biased
/// sub-queries WITHOUT needing the optional local LLM, so a fresh install (no
/// models) still fans out from the very first search.
///
/// `expand_query` only *splits* a multi-topic query on conjunctions, so a normal
/// single-topic query ("machine learning in healthcare") collapses to ONE
/// sub-query and discovery stays narrow. This adds "drop one keyword"
/// relaxations: fewer ANDed terms → more matches, while still staying on-topic
/// (each relaxation keeps all-but-one of the salient terms). Downstream ranking
/// re-tightens precision, so broadening here only raises recall. The output is
/// capped so the per-source throttle in `orchestrator::discover_wave` isn't
/// overwhelmed by a wide wave-1 fan-out.
///
/// Examples:
/// - "machine learning in healthcare" → the full phrase + "learning healthcare",
///   "machine healthcare", "machine learning".
/// - "climate change" (2 terms) → just the phrase (dropping one of two leaves a
///   lone generic word, which would pull off-topic noise).
/// - "bibles, scholarly texts, and patristic writings" → each topic's focused
///   phrase first, then per-topic relaxations.
pub fn expand_query_broad(q: &str) -> Vec<String> {
    /// Upper bound on wave-1 sub-queries. Each one fans out across every enabled
    /// source (subject to the per-source concurrency throttle), so this trades
    /// recall against per-source request volume.
    const MAX_SUBQUERIES: usize = 8;
    /// Hard ceiling on distinct topic phrases, so a pathological many-topic query
    /// (a giant pasted comma list) can't explode per-source request volume — but
    /// high enough that every topic in a realistic multi-topic query survives.
    const MAX_TOPICS: usize = 16;

    // Pre-tokenize each topical part once. Fall back to raw whitespace tokens if
    // stopword-stripping leaves nothing, so we always search *something*.
    let parsed: Vec<(String, Vec<String>)> = expand_query(q)
        .iter()
        .map(|part| {
            let kws = parse_query(part);
            let kws = if kws.is_empty() {
                part.split_whitespace().map(String::from).collect()
            } else {
                kws
            };
            (part.trim().to_string(), kws)
        })
        .collect();

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Pass 1: the focused full phrase for every topic (precision first, so a
    // truncation only ever drops relaxations — never a whole topic). Bounded by
    // MAX_TOPICS so a giant pasted list can't explode request volume.
    for (phrase, kws) in parsed.iter().take(MAX_TOPICS) {
        push_unique_subquery(phrase.clone(), kws, &mut out, &mut seen);
    }

    // Effective cap: never below MAX_SUBQUERIES, but grows to fit every topic
    // phrase so the final truncate can only ever drop drop-one RELAXATIONS — the
    // old fixed truncate(MAX_SUBQUERIES) silently dropped whole topics past the 8th.
    let cap = MAX_SUBQUERIES.max(out.len());

    // Pass 2: drop-one relaxations. Only worthwhile with >= 3 terms.
    for (_, kws) in &parsed {
        if out.len() >= cap {
            break;
        }
        if kws.len() < 3 {
            continue;
        }
        for i in 0..kws.len() {
            if out.len() >= cap {
                break;
            }
            let subset: Vec<String> = kws
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, k)| k.clone())
                .collect();
            push_unique_subquery(subset.join(" "), &subset, &mut out, &mut seen);
        }
    }

    if out.is_empty() {
        // Mirror expand_query's never-empty contract.
        out.push(q.to_string());
    }
    out.truncate(cap);
    out
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
    let base = if slug.is_empty() {
        "library".to_string()
    } else {
        slug
    };
    // Append seconds + nanoseconds so two runs of the same query within the same
    // wall-clock second still get distinct folders (and thus distinct
    // library.db files), avoiding any cross-run row collisions.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}-{}", d.as_secs(), d.subsec_nanos()))
        .unwrap_or_else(|_| "0".to_string());
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
    fn broad_expands_single_topic_with_drop_one() {
        let r = expand_query_broad("machine learning in healthcare");
        // Focused phrase is first.
        assert_eq!(r[0], "machine learning in healthcare");
        // Drop-one relaxations are present (keyword-joined, "in" is a stopword).
        assert!(r.iter().any(|s| s == "learning healthcare"));
        assert!(r.iter().any(|s| s == "machine healthcare"));
        assert!(r.iter().any(|s| s == "machine learning"));
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn broad_keeps_two_term_query_focused() {
        // Dropping one of two terms leaves a lone generic word — don't broaden.
        let r = expand_query_broad("climate change");
        assert_eq!(r, vec!["climate change"]);
    }

    #[test]
    fn broad_covers_every_topic_first() {
        let r = expand_query_broad("Christian bibles, scholarly texts, and patristic writings");
        // Each topic's focused phrase appears before any relaxation.
        assert_eq!(
            &r[0..3],
            &["Christian bibles", "scholarly texts", "patristic writings"]
        );
    }

    #[test]
    fn broad_is_capped_and_never_empty() {
        let r = expand_query_broad("alpha beta gamma delta epsilon zeta eta theta");
        assert!(r.len() <= 8, "exceeded cap: {}", r.len());
        assert!(!expand_query_broad("the of and a").is_empty());
    }

    #[test]
    fn broad_keeps_every_topic_past_eight() {
        // 10 distinct topics must ALL survive — the old fixed truncate(8) dropped
        // topics 9 and 10, leaving entire subjects unsearched.
        let q = "alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa";
        let r = expand_query_broad(q);
        for topic in [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        ] {
            assert!(r.iter().any(|s| s == topic), "missing topic {topic}: {r:?}");
        }
    }

    #[test]
    fn broad_dedups_equivalent_searches() {
        // Single keyword after stopword-strip → exactly one sub-query, no dup.
        let r = expand_query_broad("the therapy");
        assert_eq!(r, vec!["the therapy"]);
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
        // Suffix is now `{secs}-{nanos}`; the final segment is the sub-second
        // nanoseconds field (still a parseable integer).
        let suffix = slug.rsplit_once('-').unwrap().1;
        assert!(
            suffix.parse::<u64>().is_ok(),
            "non-numeric suffix: {}",
            suffix
        );
    }
}
