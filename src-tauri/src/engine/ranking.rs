//! Model-free relevance ranking for the deduplicated candidate set.
//!
//! Replaces the old TF-IDF + optional on-device embedding/LLM tiers with a
//! classic, well-understood IR stack that needs no model download:
//!
//!   * **BM25F** over title (weight 3) + abstract (1) + venue (0.5), with
//!     per-field length normalization and a light English stemmer so
//!     "networks" matches "network".
//!   * **Concept coverage** — a doc that touches every aspect of a multi-term
//!     query beats one that repeats a single term. Expansion terms give half
//!     credit, so vocabulary bridging ("myocardial infarction" for "heart
//!     attack") still counts.
//!   * **Phrase bonus** — adjacent query terms appearing together in the title.
//!   * **Cross-source agreement** — Reciprocal Rank Fusion over each source's
//!     rank plus a small bonus per distinct source.
//!   * **Authority** — per-host multiplier (see `authority.rs`).
//!   * **Citations** — log-damped `cited_by` from OpenAlex / Semantic Scholar.
//!   * **Citation-graph boost** — supplied by the caller (see `citation_graph`).
//!
//! [`expansion_terms`] implements RM3-style pseudo-relevance feedback: it
//! mines the best first-wave results for the vocabulary the literature
//! actually uses, which seeds the second discovery wave.

use super::authority::authority_multiplier;
use super::dedup::MergedDoc;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};

/// Standard RRF tuning constant (Cormack et al., 2009).
const RRF_K: f32 = 60.0;
const K1: f32 = 1.2;
const W_TITLE: f32 = 3.0;
const W_ABSTRACT: f32 = 1.0;
const W_VENUE: f32 = 0.5;
const B_TITLE: f32 = 0.5;
const B_ABSTRACT: f32 = 0.75;
const B_VENUE: f32 = 0.3;

/// A docs relevance must be at least this fraction of the best doc's to be
/// downloaded.
const RELATIVE_FLOOR: f32 = 0.10;

/// Lowercase alphanumeric tokens of >= 2 chars.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

/// Tiny suffix-stripping stemmer (an "S-stemmer" plus a few common
/// suffixes). Deliberately conservative: it only needs to make singular and
/// plural forms collide, and must never merge unrelated words.
pub fn stem(tok: &str) -> String {
    let t = tok;
    let n = t.chars().count();
    if n <= 3 || !t.is_ascii() {
        return t.to_string();
    }
    if let Some(s) = t.strip_suffix("ies") {
        if n > 4 {
            return format!("{s}y");
        }
    }
    if t.ends_with("sses") {
        return t[..t.len() - 2].to_string();
    }
    if let Some(s) = t.strip_suffix("es") {
        if s.ends_with("ch") || s.ends_with("sh") || s.ends_with('x') || s.ends_with("ss") {
            return s.to_string();
        }
    }
    if t.ends_with('s') && !t.ends_with("ss") && !t.ends_with("us") && !t.ends_with("is") {
        return t[..t.len() - 1].to_string();
    }
    t.to_string()
}

fn stems(text: &str) -> Vec<String> {
    tokenize(text).iter().map(|t| stem(t)).collect()
}

/// Weighted query terms. `original` terms came from the user; `expansion`
/// terms came from pseudo-relevance feedback.
#[derive(Debug, Clone, Default)]
pub struct QueryModel {
    original: Vec<String>,
    expansion: Vec<(String, f32)>,
    /// Adjacent original-term pairs, for the title phrase bonus.
    bigrams: Vec<(String, String)>,
}

impl QueryModel {
    /// Build from the user's query keywords (already stopword-stripped).
    pub fn from_keywords(keywords: &[String]) -> Self {
        let mut original: Vec<String> = Vec::new();
        let mut seq: Vec<String> = Vec::new();
        for k in keywords {
            for t in tokenize(k) {
                let s = stem(&t);
                seq.push(s.clone());
                if !original.contains(&s) {
                    original.push(s);
                }
            }
        }
        let bigrams = seq
            .windows(2)
            .filter(|w| w[0] != w[1])
            .map(|w| (w[0].clone(), w[1].clone()))
            .collect();
        Self {
            original,
            expansion: Vec::new(),
            bigrams,
        }
    }

    /// Add pseudo-relevance-feedback terms (surface forms are stemmed here).
    pub fn with_expansion(mut self, terms: &[(String, f32)]) -> Self {
        for (t, w) in terms {
            for tok in tokenize(t) {
                let s = stem(&tok);
                if self.original.contains(&s) || self.expansion.iter().any(|(e, _)| *e == s) {
                    continue;
                }
                self.expansion.push((s, w.clamp(0.0, 0.5)));
            }
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.original.is_empty()
    }

    fn weighted_terms(&self) -> impl Iterator<Item = (&str, f32, bool)> {
        self.original
            .iter()
            .map(|t| (t.as_str(), 1.0f32, true))
            .chain(self.expansion.iter().map(|(t, w)| (t.as_str(), *w, false)))
    }
}

#[derive(Debug, Clone)]
pub struct RankedDoc {
    pub doc: MergedDoc,
    /// BM25F relevance × coverage × phrase bonus (topical match only).
    pub relevance: f32,
    pub rrf: f32,
    pub authority: f32,
    pub score: f32,
    /// Reason the doc won't be downloaded, for UI surfacing. `None` = kept.
    pub reject_reason: Option<String>,
}

struct Fields {
    title: Vec<String>,
    abstract_: Vec<String>,
    venue: Vec<String>,
}

fn fields_of(m: &MergedDoc) -> Fields {
    Fields {
        title: stems(&m.doc.title),
        abstract_: m.doc.abstract_.as_deref().map(stems).unwrap_or_default(),
        venue: m.doc.venue.as_deref().map(stems).unwrap_or_default(),
    }
}

fn count(toks: &[String], t: &str) -> f32 {
    toks.iter().filter(|x| *x == t).count() as f32
}

/// Rank candidates. `boosts` maps a doc's primary URL to an extra multiplier
/// (citation-graph connectivity); pass an empty map when unavailable.
pub fn rank_candidates(
    query: &QueryModel,
    candidates: Vec<MergedDoc>,
    boosts: &HashMap<String, f32>,
) -> Vec<RankedDoc> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let fields: Vec<Fields> = candidates.iter().map(fields_of).collect();
    let n = fields.len() as f32;
    let avg = |f: &dyn Fn(&Fields) -> usize| -> f32 {
        (fields.iter().map(f).sum::<usize>() as f32 / n).max(1.0)
    };
    let avg_title = avg(&|f| f.title.len());
    let avg_abs = avg(&|f| f.abstract_.len());
    let avg_venue = avg(&|f| f.venue.len());

    // Document frequency for every query term.
    let mut df: HashMap<&str, f32> = HashMap::new();
    for (t, _, _) in query.weighted_terms() {
        let d = fields
            .iter()
            .filter(|f| {
                f.title
                    .iter()
                    .chain(&f.abstract_)
                    .chain(&f.venue)
                    .any(|x| x == t)
            })
            .count() as f32;
        df.insert(t, d);
    }
    let idf = |t: &str| -> f32 {
        let d = *df.get(t).unwrap_or(&0.0);
        (1.0 + (n - d + 0.5) / (d + 0.5)).ln()
    };

    // Coverage only counts query terms that occur somewhere in the pool. A
    // term no candidate contains is almost always a misspelling or a word the
    // literature doesn't use ("machien lerning"); requiring it would reject
    // every result.
    let n_orig = query
        .original
        .iter()
        .filter(|t| df.get(t.as_str()).copied().unwrap_or(0.0) > 0.0)
        .count()
        .max(1) as f32;
    let mut ranked: Vec<RankedDoc> = candidates
        .into_iter()
        .zip(fields)
        .map(|(merged, f)| {
            let norm = |len: usize, avg: f32, b: f32| (1.0 - b) + b * (len as f32 / avg);
            let nt = norm(f.title.len(), avg_title, B_TITLE);
            let na = norm(f.abstract_.len(), avg_abs, B_ABSTRACT);
            let nv = norm(f.venue.len(), avg_venue, B_VENUE);

            let mut bm25 = 0.0f32;
            let mut matched_orig = 0.0f32;
            let mut matched_exp = 0.0f32;
            for (t, w, is_orig) in query.weighted_terms() {
                let tf = W_TITLE * count(&f.title, t) / nt
                    + W_ABSTRACT * count(&f.abstract_, t) / na
                    + W_VENUE * count(&f.venue, t) / nv;
                if tf <= 0.0 {
                    continue;
                }
                if is_orig {
                    matched_orig += 1.0;
                } else {
                    matched_exp += 1.0;
                }
                bm25 += w * idf(t) * tf * (K1 + 1.0) / (tf + K1);
            }
            let coverage = ((matched_orig + 0.5 * matched_exp.min(2.0)) / n_orig).min(1.0);
            let phrase_hits = query
                .bigrams
                .iter()
                .filter(|(a, b)| f.title.windows(2).any(|w| w[0] == *a && w[1] == *b))
                .count() as f32;
            let phrase = 1.0 + (0.12 * phrase_hits).min(0.36);
            let relevance = bm25 * (0.4 + 0.6 * coverage) * phrase;

            let rrf: f32 = merged
                .source_ranks
                .iter()
                .map(|(_, rank)| 1.0 / (RRF_K + *rank as f32))
                .sum();
            let distinct_sources = merged
                .source_ranks
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<HashSet<_>>()
                .len() as f32;
            // rrf of a #1 single-source hit is ~0.0164; normalize around that.
            let agreement = 1.0
                + 0.25 * (rrf / 0.0164).min(3.0) / 3.0
                + 0.08 * (distinct_sources - 1.0).clamp(0.0, 4.0);
            let authority = authority_multiplier(&merged.doc.url);
            let cites = merged
                .doc
                .cited_by
                .map(|c| 1.0 + (0.04 * (1.0 + c as f32).ln()).min(0.3))
                .unwrap_or(1.0);
            let graph = boosts.get(&merged.doc.url).copied().unwrap_or(1.0);
            let score = relevance * agreement * authority * cites * graph;

            let reject_reason = if matched_orig == 0.0 && matched_exp == 0.0 {
                Some("Doesn't mention any of your search terms".to_string())
            } else if n_orig >= 3.0 && coverage < 0.34 {
                Some("Only matches one part of your search".to_string())
            } else {
                None
            };

            RankedDoc {
                doc: merged,
                relevance,
                rrf,
                authority,
                score,
                reject_reason,
            }
        })
        .collect();

    let top = ranked
        .iter()
        .filter(|r| r.reject_reason.is_none())
        .map(|r| r.relevance)
        .fold(0.0f32, f32::max);
    if top > 0.0 {
        for r in &mut ranked {
            if r.reject_reason.is_none() && r.relevance < top * RELATIVE_FLOOR {
                r.reject_reason = Some("Much weaker match than the best results".to_string());
            }
        }
    }

    ranked.sort_by(|a, b| {
        a.reject_reason
            .is_some()
            .cmp(&b.reject_reason.is_some())
            .then(b.score.total_cmp(&a.score))
            .then(b.rrf.total_cmp(&a.rrf))
    });
    ranked
}

static FEEDBACK_STOP: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "the",
        "and",
        "for",
        "with",
        "that",
        "this",
        "from",
        "are",
        "was",
        "were",
        "been",
        "have",
        "has",
        "had",
        "not",
        "but",
        "its",
        "their",
        "they",
        "them",
        "these",
        "those",
        "which",
        "who",
        "whom",
        "what",
        "when",
        "where",
        "while",
        "than",
        "then",
        "there",
        "here",
        "also",
        "into",
        "onto",
        "over",
        "under",
        "between",
        "among",
        "within",
        "without",
        "about",
        "after",
        "before",
        "during",
        "such",
        "can",
        "could",
        "may",
        "might",
        "will",
        "would",
        "should",
        "must",
        "our",
        "we",
        "us",
        "you",
        "your",
        "his",
        "her",
        "she",
        "him",
        "one",
        "two",
        "three",
        "first",
        "second",
        "new",
        "use",
        "used",
        "using",
        "based",
        "study",
        "studies",
        "paper",
        "article",
        "result",
        "results",
        "method",
        "methods",
        "approach",
        "show",
        "shows",
        "shown",
        "find",
        "found",
        "present",
        "presents",
        "propose",
        "proposed",
        "data",
        "analysis",
        "however",
        "therefore",
        "thus",
        "both",
        "each",
        "other",
        "more",
        "most",
        "many",
        "much",
        "some",
        "all",
        "any",
        "only",
        "well",
        "very",
        "how",
        "why",
        "is",
        "be",
        "an",
        "of",
        "in",
        "on",
        "to",
        "by",
        "as",
        "at",
        "or",
        "it",
        "et",
        "al",
        "via",
        "vs",
        "per",
        "non",
        "high",
        "low",
        "large",
        "small",
        "different",
        "various",
        "significant",
        "significantly",
        "important",
        "including",
        "include",
        "includes",
        "provide",
        "provides",
        "provided",
        "work",
        "research",
        "review",
        "role",
        "effect",
        "effects",
        "case",
        "cases",
        "number",
        "time",
        "level",
        "levels",
        "type",
        "types",
        "chapter",
        "volume",
        "journal",
        "university",
        "press",
        "edition",
        "book",
        "pdf",
        "http",
        "https",
        "www",
        "doi",
        "org",
        "com",
    ]
    .into_iter()
    .collect()
});

fn feedback_ok(tok: &str) -> bool {
    tok.chars().count() >= 3
        && !FEEDBACK_STOP.contains(tok)
        && !super::query::is_stopword(tok)
        && !tok.chars().all(|c| c.is_ascii_digit())
}

/// RM3-style pseudo-relevance feedback. Mines the top `k_docs` kept results
/// for terms (and adjacent-term phrases) that are frequent in the best
/// matches, rare in the rest of the pool, and not already in the query.
/// Returns surface forms with weights in (0, 0.5], best first.
pub fn expansion_terms(
    ranked: &[RankedDoc],
    query: &QueryModel,
    k_docs: usize,
    n_terms: usize,
) -> Vec<(String, f32)> {
    let kept: Vec<&RankedDoc> = ranked
        .iter()
        .filter(|r| r.reject_reason.is_none())
        .collect();
    if kept.len() < 3 {
        return Vec::new();
    }
    let top: Vec<&RankedDoc> = kept.iter().take(k_docs).copied().collect();
    let max_score = top.first().map(|r| r.score).unwrap_or(1.0).max(1e-6);

    // Pool-wide document frequency (by stem) for an IDF-like penalty.
    let pool_n = ranked.len() as f32;
    let mut pool_df: HashMap<String, f32> = HashMap::new();
    for r in ranked {
        let text = format!(
            "{} {}",
            r.doc.doc.title,
            r.doc.doc.abstract_.as_deref().unwrap_or("")
        );
        let uniq: HashSet<String> = stems(&text).into_iter().collect();
        for s in uniq {
            *pool_df.entry(s).or_insert(0.0) += 1.0;
        }
    }

    let mut weight: HashMap<String, f32> = HashMap::new();
    let mut doc_hits: HashMap<String, usize> = HashMap::new();
    let mut surface: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for r in &top {
        let text = format!(
            "{} {} {}",
            r.doc.doc.title,
            r.doc.doc.title,
            r.doc.doc.abstract_.as_deref().unwrap_or("")
        );
        let toks = tokenize(&text);
        if toks.is_empty() {
            continue;
        }
        let len = toks.len() as f32;
        let doc_w = r.score / max_score;
        let mut local: HashMap<String, f32> = HashMap::new();
        for (i, t) in toks.iter().enumerate() {
            if !feedback_ok(t) {
                continue;
            }
            let s = stem(t);
            if query.original.contains(&s) {
                continue;
            }
            *local.entry(s.clone()).or_insert(0.0) += 1.0;
            *surface
                .entry(s.clone())
                .or_default()
                .entry(t.clone())
                .or_insert(0) += 1;
            if let Some(next) = toks.get(i + 1) {
                if feedback_ok(next) {
                    let s2 = stem(next);
                    if !query.original.contains(&s2) && s2 != s {
                        let key = format!("{s} {s2}");
                        *local.entry(key.clone()).or_insert(0.0) += 1.5;
                        *surface
                            .entry(key)
                            .or_default()
                            .entry(format!("{t} {next}"))
                            .or_insert(0) += 1;
                    }
                }
            }
        }
        for (s, c) in local {
            *weight.entry(s.clone()).or_insert(0.0) += doc_w * c / len;
            *doc_hits.entry(s).or_insert(0) += 1;
        }
    }

    let mut scored: Vec<(String, f32)> = weight
        .into_iter()
        .filter(|(s, _)| doc_hits.get(s).copied().unwrap_or(0) >= 2)
        .map(|(s, w)| {
            let df = s
                .split(' ')
                .map(|p| *pool_df.get(p).unwrap_or(&1.0))
                .fold(f32::MAX, f32::min)
                .max(1.0);
            let idf = ((pool_n + 1.0) / df).ln().max(0.05);
            (s, w * idf)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    // Drop unigrams already covered by a chosen phrase.
    let mut chosen: Vec<(String, f32)> = Vec::new();
    for (s, w) in scored {
        if chosen.len() >= n_terms {
            break;
        }
        let covered = chosen
            .iter()
            .any(|(c, _)| c.split(' ').any(|p| p == s) || s.split(' ').any(|p| p == c));
        if covered {
            continue;
        }
        chosen.push((s, w));
    }
    let max_w = chosen.first().map(|c| c.1).unwrap_or(1.0).max(1e-6);
    chosen
        .into_iter()
        .map(|(s, w)| {
            let form = surface
                .get(&s)
                .and_then(|m| m.iter().max_by_key(|(_, c)| **c).map(|(f, _)| f.clone()))
                .unwrap_or(s);
            (form, 0.5 * (w / max_w).max(0.1))
        })
        .collect()
}

/// Build second-wave sub-queries from feedback terms: pair each strong
/// expansion with the query's most distinctive original term, plus one
/// expansion-only phrase. At most 4 queries.
pub fn feedback_subqueries(keywords: &[String], expansion: &[(String, f32)]) -> Vec<String> {
    if expansion.is_empty() || keywords.is_empty() {
        return Vec::new();
    }
    // Longest keyword is a cheap proxy for "most distinctive".
    let anchor = keywords
        .iter()
        .max_by_key(|k| k.chars().count())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let mut push = |q: String| {
        if !out.iter().any(|o: &String| o.eq_ignore_ascii_case(&q)) {
            out.push(q);
        }
    };
    let rest: Vec<&String> = keywords.iter().filter(|k| **k != anchor).collect();
    for (term, _) in expansion.iter().take(3) {
        if term.split(' ').any(|p| p.eq_ignore_ascii_case(&anchor)) {
            continue;
        }
        push(format!("{anchor} {term}"));
    }
    if !rest.is_empty() {
        let phrase: Vec<&str> = expansion.iter().take(2).map(|(t, _)| t.as_str()).collect();
        push(format!("{} {}", phrase.join(" "), rest[0]));
    }
    out.truncate(4);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Document;

    fn merged(title: &str, abstract_: Option<&str>, source: &str, rank: usize) -> MergedDoc {
        MergedDoc {
            doc: Document {
                title: title.to_string(),
                url: format!("https://example.com/{}", title.replace(' ', "-")),
                source: source.to_string(),
                abstract_: abstract_.map(String::from),
                ..Default::default()
            },
            source_ranks: vec![(source.to_string(), rank)],
        }
    }

    fn q(words: &[&str]) -> QueryModel {
        QueryModel::from_keywords(&words.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn stemmer_collides_plurals_only() {
        assert_eq!(stem("networks"), "network");
        assert_eq!(stem("studies"), "study");
        assert_eq!(stem("boxes"), "box");
        assert_eq!(stem("analysis"), "analysis");
        assert_eq!(stem("virus"), "virus");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("gas"), "gas");
    }

    #[test]
    fn title_match_beats_abstract_mention() {
        let cands = vec![
            merged(
                "An Unrelated Title",
                Some("brief mention of civil war"),
                "web",
                1,
            ),
            merged("Civil War Primary Sources", None, "web", 5),
        ];
        let ranked = rank_candidates(&q(&["civil", "war"]), cands, &HashMap::new());
        assert_eq!(ranked[0].doc.doc.title, "Civil War Primary Sources");
    }

    #[test]
    fn multi_source_agreement_wins_ties() {
        let single = merged("Single Source Match", None, "web", 1);
        let mut multi = merged("Multi Source Match", None, "web", 1);
        multi.source_ranks.push(("openalex".into(), 2));
        multi.source_ranks.push(("arxiv".into(), 4));
        let ranked = rank_candidates(&q(&["match"]), vec![single, multi], &HashMap::new());
        assert_eq!(ranked[0].doc.doc.title, "Multi Source Match");
    }

    #[test]
    fn coverage_prefers_docs_touching_every_concept() {
        let cands = vec![
            merged("Deep learning deep learning deep learning", None, "web", 1),
            merged("Deep learning for protein folding", None, "web", 2),
        ];
        let ranked = rank_candidates(&q(&["deep", "learning", "protein"]), cands, &HashMap::new());
        assert_eq!(ranked[0].doc.doc.title, "Deep learning for protein folding");
    }

    #[test]
    fn off_topic_docs_are_rejected_and_sorted_last() {
        let cands = vec![
            merged("Totally Off Topic Pottery", None, "web", 1),
            merged("Civil War Letters", None, "web", 9),
        ];
        let ranked = rank_candidates(&q(&["civil", "war"]), cands, &HashMap::new());
        assert_eq!(ranked[0].doc.doc.title, "Civil War Letters");
        assert!(ranked[0].reject_reason.is_none());
        assert!(ranked[1].reject_reason.is_some());
    }

    #[test]
    fn single_aspect_match_rejected_for_long_queries() {
        let cands = vec![
            merged("Climate change and agriculture in Africa", None, "web", 1),
            merged("Agriculture handbook", None, "web", 2),
        ];
        let ranked = rank_candidates(
            &q(&["climate", "agriculture", "africa"]),
            cands,
            &HashMap::new(),
        );
        assert!(ranked[0].reject_reason.is_none());
        assert!(ranked[1].reject_reason.is_some());
    }

    #[test]
    fn misspelled_terms_dont_reject_everything() {
        let cands = vec![
            merged(
                "Interpretability of machine learning models",
                None,
                "openalex",
                1,
            ),
            merged("A survey of model interpretability", None, "arxiv", 1),
        ];
        let ranked = rank_candidates(
            &q(&["machien", "lerning", "interpretability"]),
            cands,
            &HashMap::new(),
        );
        assert!(
            ranked.iter().all(|r| r.reject_reason.is_none()),
            "{ranked:?}"
        );
    }

    #[test]
    fn graph_boost_applies() {
        let a = merged("Graph neural networks", None, "web", 1);
        let b = merged("Graph neural networks survey", None, "web", 1);
        let mut boosts = HashMap::new();
        boosts.insert(b.doc.url.clone(), 2.0);
        let ranked = rank_candidates(&q(&["graph", "neural"]), vec![a, b], &boosts);
        assert_eq!(ranked[0].doc.doc.title, "Graph neural networks survey");
    }

    #[test]
    fn empty_candidates_produces_empty() {
        assert!(rank_candidates(&q(&["foo"]), vec![], &HashMap::new()).is_empty());
    }

    #[test]
    fn feedback_finds_shared_vocabulary() {
        let abs = "Outcomes after acute myocardial infarction in older patients";
        let cands = vec![
            merged("Heart attack treatment outcomes", Some(abs), "openalex", 1),
            merged("Heart attack in women", Some(abs), "openalex", 2),
            merged("Treating heart attack fast", Some(abs), "europe_pmc", 1),
            merged(
                "Heart attack registry",
                Some("national registry"),
                "openalex",
                3,
            ),
            merged("Gardening tips", Some("roses and tulips"), "web", 1),
        ];
        let qm = q(&["heart", "attack"]);
        let ranked = rank_candidates(&qm, cands, &HashMap::new());
        let exp = expansion_terms(&ranked, &qm, 10, 6);
        assert!(
            exp.iter().any(|(t, _)| t.contains("myocardial")),
            "expansion was {exp:?}"
        );
        let subs = feedback_subqueries(&["heart".into(), "attack".into()], &exp);
        assert!(!subs.is_empty() && subs.len() <= 4);
    }
}
