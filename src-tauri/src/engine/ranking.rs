//! Lexical ranking for the deduplicated candidate set.
//!
//! Three independent signals get combined into a single score:
//!
//!   * **TF-IDF** over the candidate corpus — measures how well each
//!     document matches the query relative to how distinctive the query
//!     terms are within the candidates pool. Strong signal even with
//!     short titles + abstracts.
//!   * **Reciprocal Rank Fusion** — when a paper surfaces from multiple
//!     sources, we use each source's per-source rank to compute
//!     `Σ 1/(k + rank)`. Standard k=60 default. A paper that's #1 in
//!     arXiv and #3 in Semantic Scholar gets a much higher RRF score
//!     than one that's #80 in a single source.
//!   * **Domain authority** — flat multiplier per host, see `authority.rs`.
//!
//! Final score: `(rrf + epsilon) * (tfidf + epsilon) * authority`.
//! The `epsilon` keeps zero scores in one dimension from collapsing the
//! whole product.

use super::authority::authority_multiplier;
use super::dedup::MergedDoc;
use std::collections::HashMap;

/// Standard RRF tuning constant. The paper that originated RRF
/// (Cormack et al., 2009) found k≈60 across many TREC datasets.
const RRF_K: f32 = 60.0;

/// Used to keep zero-component scores from collapsing the product.
const EPSILON: f32 = 0.001;

/// Title term-frequency boost — title matches count this many times more
/// than abstract matches because titles are densely informative.
const TITLE_BOOST: f32 = 3.0;

/// Tokenize a string into lowercased word tokens. Drops single-character noise
/// but keeps short acronyms (e.g. "AI", "ML") — filtering by char count, not
/// byte length, so multi-byte 2-char tokens aren't mis-measured. (A byte-length
/// `>= 3` filter silently dropped every token of an all-2-char query, zeroing
/// the whole TF-IDF column and rejecting every result.)
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

#[derive(Debug, Clone)]
pub struct RankedDoc {
    pub doc: MergedDoc,
    pub tfidf: f32,
    pub rrf: f32,
    pub authority: f32,
    pub score: f32,
    /// Reason the doc is borderline / rejected, for UI surfacing.
    /// `None` when the doc is selected for download.
    pub reject_reason: Option<String>,
}

/// Rank deduplicated candidates against the query keywords. Returns
/// every input doc ordered by descending score; downstream tiers can
/// filter or rerank further.
pub fn rank_candidates(query_terms: &[String], candidates: Vec<MergedDoc>) -> Vec<RankedDoc> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let q_tokens: Vec<String> = query_terms
        .iter()
        .flat_map(|t| tokenize(t))
        .collect::<Vec<_>>();

    // ----- Build IDF over the candidate corpus -------------------------
    let n_docs = candidates.len() as f32;
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut tokenized: Vec<(Vec<String>, Vec<String>)> = Vec::with_capacity(candidates.len());
    for c in &candidates {
        let title_tokens = tokenize(&c.doc.title);
        let abstract_tokens = c.doc.abstract_.as_deref().map(tokenize).unwrap_or_default();
        // Document frequency: each unique token in the doc counts once.
        let mut seen_in_doc: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for tok in title_tokens.iter().chain(abstract_tokens.iter()) {
            if seen_in_doc.insert(tok.as_str()) {
                *df.entry(tok.clone()).or_insert(0) += 1;
            }
        }
        tokenized.push((title_tokens, abstract_tokens));
    }
    let idf = |term: &str| -> f32 {
        let dfi = *df.get(term).unwrap_or(&0) as f32;
        // Add-one smoothing avoids -inf for unseen terms; +1 keeps IDF positive.
        ((n_docs + 1.0) / (dfi + 1.0)).ln() + 1.0
    };

    // ----- Score each doc ---------------------------------------------
    let mut ranked: Vec<RankedDoc> = candidates
        .into_iter()
        .zip(tokenized)
        .map(|(merged, (title_toks, abs_toks))| {
            let mut tfidf = 0.0f32;
            for q in &q_tokens {
                let title_tf = title_toks.iter().filter(|t| *t == q).count() as f32;
                let abs_tf = abs_toks.iter().filter(|t| *t == q).count() as f32;
                let weighted_tf = TITLE_BOOST * title_tf + abs_tf;
                tfidf += weighted_tf * idf(q);
            }

            let rrf: f32 = merged
                .source_ranks
                .iter()
                .map(|(_, rank)| 1.0 / (RRF_K + *rank as f32))
                .sum();

            let authority = authority_multiplier(&merged.doc.url);

            let score = (tfidf + EPSILON) * (rrf + EPSILON) * authority;

            RankedDoc {
                doc: merged,
                tfidf,
                rrf,
                authority,
                score,
                reject_reason: None,
            }
        })
        .collect();

    // Stable sort by score descending. Ties broken by RRF (more sources wins),
    // then by authority.
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.rrf
                    .partial_cmp(&a.rrf)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b.authority
                    .partial_cmp(&a.authority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    ranked
}

/// Annotate candidates with `reject_reason` when their TF-IDF is so low
/// the document almost certainly doesn't address the query. The threshold
/// is the larger of an absolute floor and a fraction of the top score —
/// "very weak match in absolute terms AND much weaker than the best".
pub fn flag_rejects(mut ranked: Vec<RankedDoc>) -> Vec<RankedDoc> {
    if ranked.is_empty() {
        return ranked;
    }
    let top_tfidf = ranked.iter().map(|r| r.tfidf).fold(0.0f32, f32::max);
    // A uniformly-zero TF-IDF column (e.g. a query whose only tokens were too
    // short to tokenize) carries no signal — rejecting on it would flag every
    // doc and the run would download nothing. Leave the results unflagged.
    if top_tfidf == 0.0 {
        return ranked;
    }
    // Floors kept deliberately permissive so the gate removes only true
    // zero/near-zero-overlap noise — a rejected doc is excluded from downloads
    // entirely (orchestrator filters reject_reason before the download phase),
    // so over-aggressive floors directly cost relevant papers on broad queries.
    let absolute_floor = 0.05f32;
    let relative_floor = top_tfidf * 0.02;
    let cutoff = absolute_floor.max(relative_floor);

    for r in &mut ranked {
        if r.tfidf < cutoff {
            r.reject_reason = Some(format!("TF-IDF {:.2} below cutoff {:.2}", r.tfidf, cutoff));
        }
    }
    ranked
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
                authors: vec![],
                year: None,
                abstract_: abstract_.map(String::from),
                identifier: None,
            },
            source_ranks: vec![(source.to_string(), rank)],
        }
    }

    #[test]
    fn ranking_prefers_title_match_over_abstract_match() {
        let cands = vec![
            merged("Civil War Primary Sources", None, "web", 5),
            merged(
                "An Unrelated Title",
                Some("brief mention of civil war"),
                "web",
                1,
            ),
        ];
        let ranked = rank_candidates(&["civil".into(), "war".into()], cands);
        assert_eq!(ranked[0].doc.doc.title, "Civil War Primary Sources");
    }

    #[test]
    fn rrf_rewards_multi_source_appearance() {
        let mut single = merged("Same Paper", None, "web", 1);
        let mut multi = merged("Same Paper", None, "web", 1);
        multi.source_ranks.push(("brave".into(), 2));
        multi.source_ranks.push(("bing".into(), 4));
        // Different titles to avoid TF-IDF tie; equal lexical match.
        single.doc.title = "Single Source Match".into();
        multi.doc.title = "Multi Source Match".into();
        let ranked = rank_candidates(&["match".into()], vec![single.clone(), multi.clone()]);
        assert_eq!(ranked[0].doc.doc.title, "Multi Source Match");
    }

    #[test]
    fn empty_candidates_produces_empty() {
        let r = rank_candidates(&["foo".into()], vec![]);
        assert!(r.is_empty());
    }

    #[test]
    fn flag_rejects_marks_far_below_cutoff() {
        let cands = vec![
            merged("Many Civil War Civil War Primary", None, "web", 1),
            merged("Totally Off Topic Pottery", None, "web", 1),
        ];
        let ranked = flag_rejects(rank_candidates(&["civil".into(), "war".into()], cands));
        assert!(ranked.iter().any(|r| r.reject_reason.is_some()));
    }
}
