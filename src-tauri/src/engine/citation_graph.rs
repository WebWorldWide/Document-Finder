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
//! Always on, but bounded: it runs alongside downloads (never gating them),
//! fetches at most 4 papers at a time, and gives up after a short budget —
//! whatever finished in time re-orders the remaining download queue.

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

/// Process-wide memoization of S2 fetches keyed by `(endpoint_kind, doi)`.
/// Refs/cites are immutable once published (papers don't un-cite each other),
/// so any later query that looks at the same DOI gets a free hit. Capped at
/// `FETCH_CACHE_MAX` entries (cleared wholesale on overflow) so a long-lived
/// session running many distinct queries can't grow it without bound.
type FetchCache = RwLock<HashMap<(&'static str, String), Vec<String>>>;
static FETCH_CACHE: Lazy<FetchCache> = Lazy::new(|| RwLock::new(HashMap::new()));
/// Upper bound on memoized `(kind, doi)` entries before the cache is cleared.
const FETCH_CACHE_MAX: usize = 4096;

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
        // Bound process-lifetime growth: clear wholesale on overflow rather than
        // tracking per-entry LRU — refs/cites are only reused within a run's
        // top-K fanout, so the occasional cold start after a clear is cheap.
        if g.len() >= FETCH_CACHE_MAX {
            g.clear();
        }
        g.insert(key, dois.clone());
    }
    dois
}

/// Citation-graph connectivity boosts for the current top candidates.
///
/// `top` is `(primary_url, doi)` for up to [`TOP_K`] best candidates. For each
/// we fetch Semantic Scholar references + citations (4 papers at a time, to
/// stay inside the public rate limit) and count how many *other* top
/// candidates it links to. Returns `url -> multiplier` (1.1 … 1.5). Bounded by
/// `budget`; whatever finished in time is used.
pub async fn citation_boosts(
    client: Arc<reqwest::Client>,
    top: Vec<(String, String)>,
    budget: std::time::Duration,
) -> HashMap<String, f32> {
    use futures::StreamExt;
    let top: Vec<(String, String)> = top
        .into_iter()
        .take(TOP_K)
        .map(|(u, d)| (u, d.to_lowercase()))
        .collect();
    let doi_to_url: HashMap<String, String> =
        top.iter().map(|(u, d)| (d.clone(), u.clone())).collect();
    if doi_to_url.len() < 2 {
        return HashMap::new();
    }
    let mut intra: HashMap<String, usize> = HashMap::new();
    let work = futures::stream::iter(top.clone())
        .map(|(url, doi)| {
            let client = client.clone();
            async move {
                let (mut refs, cites) = tokio::join!(
                    fetch_dois(&client, REFERENCES_URL, &doi),
                    fetch_dois(&client, CITATIONS_URL, &doi),
                );
                refs.extend(cites);
                (url, refs)
            }
        })
        .buffer_unordered(4);
    tokio::pin!(work);
    let deadline = tokio::time::sleep(budget);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            next = work.next() => {
                let Some((src_url, related)) = next else { break };
                for d in related {
                    if let Some(target) = doi_to_url.get(&d.to_lowercase()) {
                        if *target != src_url {
                            *intra.entry(target.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    intra
        .into_iter()
        .map(|(u, c)| {
            (
                u,
                1.0 + PER_CITATION_BOOST * c.min(MAX_BOOST_CITATIONS) as f32,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fewer_than_two_dois_is_a_no_op() {
        let client = Arc::new(reqwest::Client::new());
        let out = citation_boosts(
            client,
            vec![("https://a".into(), "10.1/x".into())],
            std::time::Duration::from_millis(10),
        )
        .await;
        assert!(out.is_empty());
    }
}
