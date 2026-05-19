//! Citation-graph reasoning over the top-ranked candidate set.
//!
//! Queries Semantic Scholar's references + citations endpoints for each
//! candidate that has a DOI, builds an in-memory graph of which top
//! candidates cite which other top candidates, and applies a multiplicative
//! score boost to nodes with high in-degree from the candidate set itself.
//!
//! The reasoning is "papers trusted by other relevant papers". A paper that
//! is independently cited by three of your top-ten matches is much more
//! likely to be a foundational reference than one that just happens to
//! match your query terms in isolation.
//!
//! Off by default — opt-in via `RunRequest.use_citation_graph`. Adds 30+
//! API calls and is rate-limited by Semantic Scholar; only enable for
//! research-grade queries where the extra second matters.

use super::ranking::RankedDoc;
use crate::sources::Document;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Top-K candidates to enrich. Higher K means more API calls (slower) but
/// more graph signal. 30 is a reasonable balance for typical research runs.
const TOP_K: usize = 30;

/// Per-candidate fanout when fetching refs/cites. S2 supports up to 1000
/// but we cap aggressively to keep latency bounded.
const FANOUT: usize = 50;

const REFERENCES_URL: &str = "https://api.semanticscholar.org/graph/v1/paper/{}/references";
const CITATIONS_URL: &str = "https://api.semanticscholar.org/graph/v1/paper/{}/citations";

/// Score multiplier per candidate-set in-citation. Capped to avoid runaway.
const PER_CITATION_BOOST: f32 = 0.10;
/// Cap on the number of in-citations counted — past this, additional ones
/// don't help (otherwise a single survey paper would dominate).
const MAX_BOOST_CITATIONS: usize = 5;

#[derive(Debug, Deserialize)]
struct RefsResp {
    #[serde(default)]
    data: Vec<RefEntry>,
}

#[derive(Debug, Deserialize)]
struct RefEntry {
    #[serde(default, rename = "citedPaper")]
    cited_paper: Option<PaperRef>,
    #[serde(default, rename = "citingPaper")]
    citing_paper: Option<PaperRef>,
}

#[derive(Debug, Deserialize)]
struct PaperRef {
    #[serde(default, rename = "externalIds")]
    external_ids: Option<ExternalIds>,
}

#[derive(Debug, Deserialize, Default)]
struct ExternalIds {
    #[serde(default, rename = "DOI")]
    doi: Option<String>,
}

fn doi_from(doc: &Document) -> Option<String> {
    doc.identifier
        .as_deref()
        .and_then(super::dedup::extract_doi)
        .or_else(|| super::dedup::extract_doi(&doc.url))
}

/// Process-wide memoization of S2 fetches keyed by `(endpoint_kind, doi)`.
/// Refs/cites are immutable once published (papers don't un-cite each other),
/// so any later query that looks at the same DOI gets a free hit. Bounded by
/// unique DOIs ever queried this process — practically a few hundred.
static FETCH_CACHE: Lazy<RwLock<HashMap<(&'static str, String), Vec<String>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

async fn fetch_dois(client: &reqwest::Client, template: &str, paper_doi: &str) -> Vec<String> {
    let kind = if template == REFERENCES_URL {
        "refs"
    } else {
        "cites"
    };
    let key = (kind, paper_doi.to_string());
    if let Ok(g) = FETCH_CACHE.read() {
        if let Some(hit) = g.get(&key) {
            return hit.clone();
        }
    }

    let endpoint = template.replace("{}", &format!("DOI:{}", paper_doi));
    let resp = client
        .get(&endpoint)
        .query(&[("fields", "externalIds"), ("limit", &FANOUT.to_string())])
        .send()
        .await;
    let Ok(resp) = resp else { return Vec::new() };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(parsed) = resp.json::<RefsResp>().await else {
        return Vec::new();
    };
    let dois: Vec<String> = parsed
        .data
        .into_iter()
        .filter_map(|e| {
            let p = e.cited_paper.or(e.citing_paper)?;
            p.external_ids?.doi
        })
        .map(|d| d.to_lowercase())
        .collect();
    if let Ok(mut g) = FETCH_CACHE.write() {
        g.insert(key, dois.clone());
    }
    dois
}

/// Apply a citation-graph boost to the top K of `ranked` and return the
/// re-sorted list. Candidates without DOIs are unaffected. Errors from the
/// S2 API are silently absorbed — the worst case is no boost, not a
/// pipeline failure.
pub async fn enrich_with_citation_graph(
    client: Arc<reqwest::Client>,
    mut ranked: Vec<RankedDoc>,
) -> Vec<RankedDoc> {
    if ranked.is_empty() {
        return ranked;
    }

    // Build a (top-K) DOI → index lookup so we can detect intra-set citations.
    let top_indices: Vec<usize> = ranked
        .iter()
        .take(TOP_K)
        .enumerate()
        .map(|(i, _)| i)
        .collect();
    let mut doi_to_index: HashMap<String, usize> = HashMap::new();
    for &i in &top_indices {
        if let Some(d) = doi_from(&ranked[i].doc.doc) {
            doi_to_index.insert(d.to_lowercase(), i);
        }
    }
    if doi_to_index.is_empty() {
        return ranked;
    }

    // Concurrently fetch references + citations for each top-K candidate
    // that has a DOI. Within each task the two endpoint calls run in
    // parallel via tokio::join so an N-task fanout becomes 2N concurrent
    // HTTP requests instead of 2 serial requests × N tasks.
    let mut tasks: tokio::task::JoinSet<(usize, Vec<String>)> = tokio::task::JoinSet::new();
    for &i in &top_indices {
        let Some(my_doi) = doi_from(&ranked[i].doc.doc) else {
            continue;
        };
        let client_r = client.clone();
        let client_c = client.clone();
        let doi_r = my_doi.clone();
        let doi_c = my_doi.clone();
        tasks.spawn(async move {
            let (mut refs, cites) = tokio::join!(
                fetch_dois(&client_r, REFERENCES_URL, &doi_r),
                fetch_dois(&client_c, CITATIONS_URL, &doi_c),
            );
            refs.extend(cites);
            (i, refs)
        });
    }

    // Tally how many top-K candidates each top-K candidate is connected to.
    let mut intra_citations: HashMap<usize, usize> = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        let Ok((source_idx, related)) = res else {
            continue;
        };
        for d in related {
            if let Some(&target_idx) = doi_to_index.get(&d) {
                if target_idx != source_idx {
                    *intra_citations.entry(target_idx).or_insert(0) += 1;
                }
            }
        }
    }

    // Apply the boost.
    for (idx, count) in intra_citations {
        let bounded = count.min(MAX_BOOST_CITATIONS);
        let mult = 1.0 + PER_CITATION_BOOST * bounded as f32;
        ranked[idx].score *= mult;
        ranked[idx].authority *= mult;
    }

    // Re-sort because boosts changed scores.
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::dedup::MergedDoc;

    fn ranked_doi(score: f32, doi: &str) -> RankedDoc {
        RankedDoc {
            doc: MergedDoc {
                doc: Document {
                    title: format!("Paper {}", doi),
                    url: format!("https://example.com/{}", doi),
                    source: "test".into(),
                    authors: vec![],
                    year: None,
                    abstract_: None,
                    identifier: Some(doi.to_string()),
                },
                source_ranks: vec![("test".into(), 1)],
            },
            tfidf: 1.0,
            rrf: 0.01,
            authority: 1.0,
            score,
            reject_reason: None,
        }
    }

    #[test]
    fn doi_from_identifier_field() {
        let r = ranked_doi(1.0, "10.1038/nature12373");
        assert_eq!(
            doi_from(&r.doc.doc),
            Some("10.1038/nature12373".to_string())
        );
    }

    // Network-dependent tests skipped — exercised by integration / manual UAT.
}
