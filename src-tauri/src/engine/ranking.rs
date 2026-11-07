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

/// A doc's relevance must be at least this fraction of the typical strong
/// match (median of the top 20 kept) to be downloaded.
const RELATIVE_FLOOR: f32 = 0.3;

/// Lowercase alphanumeric tokens of >= 2 chars.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

/// Generic research words: they shape a query ("treatment outcomes",
/// "effects of …") but can never be what makes a document relevant. A
/// document must match the query's *content* words; these only add score.
/// Stored stemmed.
static GENERIC_TERMS: Lazy<HashSet<String>> = Lazy::new(|| {
    [
        "treatment",
        "outcome",
        "effect",
        "impact",
        "role",
        "study",
        "analysis",
        "review",
        "method",
        "methodology",
        "model",
        "modeling",
        "modelling",
        "approach",
        "system",
        "use",
        "using",
        "application",
        "development",
        "evaluation",
        "performance",
        "management",
        "risk",
        "factor",
        "research",
        "overview",
        "introduction",
        "theory",
        "practice",
        "case",
        "trend",
        "future",
        "challenge",
        "new",
        "recent",
        "current",
        "modern",
        "based",
        "data",
        "information",
        "process",
        "design",
        "problem",
        "issue",
        "survey",
        "guide",
        "basic",
        "comparison",
        "relationship",
        "influence",
        "association",
        "history",
        "benefit",
        "cause",
        "type",
        "level",
        "change",
        "evidence",
        "assessment",
        "strategy",
        "policy",
        "result",
        "finding",
        "health",
        "care",
        "patient",
        "people",
        "human",
        "effectiveness",
        "efficacy",
        "prevention",
        "intervention",
        "technique",
        "technology",
        "tool",
        "framework",
        "aspect",
        "perspective",
        "example",
        "importance",
        "role",
    ]
    .iter()
    .map(|w| stem(w))
    .collect()
});

/// True for generic research words (see [`GENERIC_TERMS`]); input is a stem.
pub fn is_generic(stemmed: &str) -> bool {
    GENERIC_TERMS.contains(stemmed)
}

/// Split query terms into the content words a document must match and the
/// generic modifiers that only add score. If every term is generic
/// ("data analysis"), all of them count as content.
pub fn content_terms(terms: &[String]) -> Vec<String> {
    let content: Vec<String> = terms.iter().filter(|t| !is_generic(t)).cloned().collect();
    if content.is_empty() {
        terms.to_vec()
    } else {
        content
    }
}

/// Model-free synonym table for common research vocabulary. Each group is a
/// set of interchangeable phrasings; matching any one satisfies a concept
/// that uses another. Entries are surface text (stemmed when loaded).
const SYNONYM_GROUPS: &[&[&str]] = &[
    &[
        "heart attack",
        "myocardial infarction",
        "acute coronary syndrome",
    ],
    &[
        "teenager",
        "adolescent",
        "adolescence",
        "teen",
        "youth",
        "high school student",
    ],
    &[
        "child",
        "children",
        "kid",
        "pediatric",
        "paediatric",
        "childhood",
    ],
    &[
        "elderly",
        "older adult",
        "older people",
        "aged",
        "geriatric",
        "senior",
    ],
    &["woman", "women", "female"],
    &["man", "men", "male"],
    &[
        "cancer",
        "tumor",
        "tumour",
        "oncology",
        "carcinoma",
        "neoplasm",
        "malignancy",
    ],
    &["car", "vehicle", "automobile"],
    &["electric car", "electric vehicle", "ev"],
    &["ai", "artificial intelligence"],
    &["economy", "economic", "economics"],
    &["climate change", "global warming"],
    &["covid", "coronavirus", "sars cov"],
    &["stroke", "cerebrovascular"],
    &["diabetes", "diabetic"],
    &["obesity", "obese", "overweight"],
    &["depression", "depressive"],
    &["anxiety", "anxious"],
    &["usa", "united states", "america"],
    &["uk", "united kingdom", "britain", "british"],
    &["solar panel", "photovoltaic", "solar cell", "solar module"],
    &["mouse", "mice", "murine"],
    &["tooth", "teeth", "dental"],
    &["kidney", "renal"],
    &["liver", "hepatic"],
    &["lung", "pulmonary"],
    &["brain", "cerebral", "neural"],
    &["medicine", "medical", "clinical"],
    &["drug", "medication", "pharmaceutical"],
    &["school", "education", "educational"],
    &["student", "pupil", "learner"],
    &["memory", "memorization", "recall"],
    &["sleep", "sleeping"],
    &["exercise", "physical activity", "training"],
    &["diet", "dietary", "nutrition"],
    &["water", "aquatic"],
    &["ocean", "marine", "sea"],
    &["plant", "botanical", "vegetation"],
    &["city", "urban"],
    &["farm", "agriculture", "agricultural"],
];

static SYNONYMS: Lazy<HashMap<String, Vec<Vec<String>>>> = Lazy::new(|| {
    let mut m: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for group in SYNONYM_GROUPS {
        let forms: Vec<Vec<String>> = group.iter().map(|g| stems(g)).collect();
        for f in &forms {
            let key = f.join(" ");
            let entry = m.entry(key).or_default();
            for g in &forms {
                if g != f && !entry.contains(g) {
                    entry.push(g.clone());
                }
            }
        }
    }
    m
});

/// One thing the user asked about: a single word or a phrase ("heart
/// attack"), plus equivalent phrasings from the synonym table.
#[derive(Debug, Clone, PartialEq)]
pub struct Concept {
    pub label: String,
    /// Every accepted phrasing (the user's own first), as stem sequences.
    pub forms: Vec<Vec<String>>,
}

impl Concept {
    fn new(terms: Vec<String>) -> Self {
        let label = terms.join(" ");
        let mut forms = vec![terms];
        if let Some(alts) = SYNONYMS.get(&label) {
            forms.extend(alts.iter().cloned());
        }
        Self { label, forms }
    }

    /// Occurrences of any phrasing in a stem sequence.
    pub fn count_in(&self, toks: &[String]) -> usize {
        self.forms
            .iter()
            .map(|f| {
                if f.len() == 1 {
                    toks.iter().filter(|t| **t == f[0]).count()
                } else {
                    toks.windows(f.len()).filter(|w| *w == f.as_slice()).count()
                }
            })
            .sum()
    }
}

/// Concepts the user is asking about, derived from the query's content words
/// and the candidate pool. Two adjacent query words are treated as one
/// phrase when the pool mostly uses them side by side ("heart attack",
/// "Roman empire"); a phrase with a known synonym ("myocardial infarction")
/// accepts that too. `pool` is each candidate's title+abstract stems.
pub fn concepts_for(query: &QueryModel, pool: &[Vec<String>]) -> Vec<Concept> {
    let present = |t: &str| pool.iter().any(|toks| toks.iter().any(|x| x == t));
    let has_synonym = |t: &str| SYNONYMS.contains_key(t);
    // Content words, in query order, that the pool uses (or that have a
    // synonym the pool might use instead).
    let ordered: Vec<String> = {
        let mut seen = HashSet::new();
        query
            .original
            .iter()
            .filter(|t| seen.insert((*t).clone()))
            .cloned()
            .collect()
    };
    let content: Vec<String> = content_terms(&ordered)
        .into_iter()
        .filter(|t| present(t) || has_synonym(t))
        .collect();
    let mut out: Vec<Concept> = Vec::new();
    let mut i = 0;
    while i < content.len() {
        if i + 1 < content.len() {
            let (a, b) = (&content[i], &content[i + 1]);
            let pair = format!("{a} {b}");
            let (mut both, mut adjacent) = (0usize, 0usize);
            for toks in pool {
                let has_a = toks.iter().any(|x| x == a);
                let has_b = toks.iter().any(|x| x == b);
                if has_a && has_b {
                    both += 1;
                    if toks.windows(2).any(|w| w[0] == *a && w[1] == *b) {
                        adjacent += 1;
                    }
                }
            }
            let is_phrase = SYNONYMS.contains_key(&pair) || (both >= 3 && adjacent * 2 >= both);
            if is_phrase {
                out.push(Concept::new(vec![a.clone(), b.clone()]));
                i += 2;
                continue;
            }
        }
        out.push(Concept::new(vec![content[i].clone()]));
        i += 1;
    }
    out
}

/// Concepts a title + abstract must cover to be downloaded: all of them,
/// or all but one once there are three or more (abstracts are short and
/// paraphrase). The full-text check after download is stricter.
fn required(n: usize) -> usize {
    if n >= 3 {
        n - 1
    } else {
        n
    }
}

/// Mentions of the topic per 10,000 words a document must reach: a paper
/// *about* the topic keeps returning to it; one that cites it in passing
/// doesn't.
const MIN_DENSITY_PER_10K: usize = 5;

/// Full-text topicality check for a downloaded document — the final gate.
/// Every concept must appear, most of them repeatedly, with at least three
/// mentions per concept overall — dense enough that the topic is what the
/// document is about, not something it cites in passing.
pub fn text_on_topic(text: &str, concepts: &[Concept]) -> bool {
    if concepts.is_empty() {
        return true;
    }
    // Bound the work on long books (~65k words).
    let sample: String = text.chars().take(400_000).collect();
    let toks = stems(&sample);
    let counts: Vec<usize> = concepts.iter().map(|c| c.count_in(&toks)).collect();
    let total: usize = counts.iter().sum();
    let words = toks.len().max(1);
    let repeated = counts.iter().filter(|c| **c >= 2).count();
    counts.iter().all(|c| *c >= 1)
        && repeated >= required(concepts.len())
        && total >= 3 * concepts.len()
        && total * 10_000 >= MIN_DENSITY_PER_10K * words
}

/// Title+abstract check for documents with no readable text: every concept.
pub fn metadata_on_topic(text: &str, concepts: &[Concept]) -> bool {
    let toks = stems(text);
    concepts.iter().all(|c| c.count_in(&toks) > 0)
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

pub fn stems(text: &str) -> Vec<String> {
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

    /// The user's own (stemmed) query terms.
    pub fn original_terms(&self) -> &[String] {
        &self.original
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
    let active: Vec<&str> = query
        .original
        .iter()
        .map(String::as_str)
        .filter(|t| df.get(t).copied().unwrap_or(0.0) > 0.0)
        .collect();
    let n_orig = active.len().max(1) as f32;
    let pool_toks: Vec<Vec<String>> = fields
        .iter()
        .map(|f| f.title.iter().chain(&f.abstract_).cloned().collect())
        .collect();
    let concepts = concepts_for(query, &pool_toks);
    let need = required(concepts.len());
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

            // Relevance gate: the title/abstract must cover the concepts the
            // user asked about (phrases kept together, synonyms accepted;
            // generic words like "treatment" or "effects" don't count).
            // Feedback terms help ranking but never make a document relevant.
            let toks: Vec<String> = f.title.iter().chain(&f.abstract_).cloned().collect();
            let covered = concepts.iter().filter(|c| c.count_in(&toks) > 0).count();
            // A title that doesn't name the topic at all means the abstract
            // must keep returning to it — otherwise the topic is incidental
            // (a statistics paper that uses heart-attack data as an example).
            let title_hits: usize = concepts.iter().map(|c| c.count_in(&f.title)).sum();
            let abstract_hits: usize = concepts.iter().map(|c| c.count_in(&f.abstract_)).sum();
            let incidental =
                !concepts.is_empty() && title_hits == 0 && abstract_hits < 2 * concepts.len();
            let reject_reason = if matched_orig == 0.0 || (!concepts.is_empty() && covered == 0) {
                Some("Doesn't mention your search terms".to_string())
            } else if covered < need {
                Some("Only matches one part of your search".to_string())
            } else if incidental {
                Some("Only mentions your topic in passing".to_string())
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

    // Floor relative to a *typical* strong match (median of the top 20), so
    // one outlier can't reject everything and a weak pool can't keep junk.
    let mut top_rel: Vec<f32> = ranked
        .iter()
        .filter(|r| r.reject_reason.is_none())
        .map(|r| r.relevance)
        .collect();
    top_rel.sort_by(|a, b| b.total_cmp(a));
    top_rel.truncate(20);
    let reference = top_rel.get(top_rel.len() / 2).copied().unwrap_or(0.0);
    if reference > 0.0 {
        for r in &mut ranked {
            if r.reject_reason.is_none() && r.relevance < reference * RELATIVE_FLOOR {
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

/// Optimal-string-alignment distance (Levenshtein + adjacent transposition),
/// bailing out early once it exceeds `max`.
fn osa_distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        let mut row_min = usize::MAX;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return max + 1;
        }
    }
    d[n][m]
}

/// Model-free spelling correction: a query word that is rare or absent in
/// the results, with a close (edit distance 1–2) word that is common in
/// them, is almost certainly a typo for it ("machien" → "machine").
/// Returns `(typo, correction)` pairs.
pub fn spell_corrections(keywords: &[String], pool: &[MergedDoc]) -> Vec<(String, String)> {
    let mut vocab: HashMap<String, usize> = HashMap::new();
    for m in pool {
        let text = format!(
            "{} {}",
            m.doc.title,
            m.doc.abstract_.as_deref().unwrap_or("")
        );
        let uniq: HashSet<String> = tokenize(&text).into_iter().collect();
        for t in uniq {
            if t.chars().count() >= 3 && t.chars().all(char::is_alphabetic) {
                *vocab.entry(t).or_insert(0) += 1;
            }
        }
    }
    let n = pool.len().max(1);
    let mut out = Vec::new();
    for k in keywords {
        let k = k.to_lowercase();
        if k.chars().count() < 4 || !k.chars().all(char::is_alphabetic) {
            continue;
        }
        let k_df = vocab.get(&k).copied().unwrap_or(0);
        if k_df * 5 >= n {
            continue; // a fifth of the results use it: not a typo
        }
        let max_d = if k.chars().count() <= 5 { 1 } else { 2 };
        let best = vocab
            .iter()
            .filter(|(w, c)| **c >= 3.max(k_df * 4) && **w != k)
            .filter_map(|(w, c)| {
                let d = osa_distance(&k, w, max_d);
                (d <= max_d).then_some((w, *c, d))
            })
            .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
        if let Some((w, _, _)) = best {
            out.push((k.clone(), w.clone()));
        }
    }
    out
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
    fn two_word_queries_need_both_words() {
        let cands = vec![
            merged("Heart attack outcomes in older adults", None, "openalex", 1),
            merged("Detecting cyber attack traffic", None, "arxiv", 1),
            merged("Heart rate variability", None, "arxiv", 2),
        ];
        let ranked = rank_candidates(&q(&["heart", "attack"]), cands, &HashMap::new());
        let kept: Vec<&str> = ranked
            .iter()
            .filter(|r| r.reject_reason.is_none())
            .map(|r| r.doc.doc.title.as_str())
            .collect();
        assert_eq!(kept, vec!["Heart attack outcomes in older adults"]);
    }

    #[test]
    fn expansion_terms_alone_never_make_a_doc_relevant() {
        let qm = q(&["heart", "attack"]).with_expansion(&[("myocardial".into(), 0.5)]);
        let cands = vec![
            merged("Heart attack registry", None, "openalex", 1),
            merged("Myocardial tissue engineering", None, "openalex", 2),
        ];
        let ranked = rank_candidates(&qm, cands, &HashMap::new());
        assert!(ranked
            .iter()
            .find(|r| r.doc.doc.title.starts_with("Myocardial"))
            .unwrap()
            .reject_reason
            .is_some());
    }

    #[test]
    fn spelling_is_corrected_from_the_results() {
        let docs: Vec<MergedDoc> = (0..6)
            .map(|i| {
                merged(
                    &format!("Machine learning interpretability study {i}"),
                    Some("interpretable machine learning models"),
                    "openalex",
                    i + 1,
                )
            })
            .collect();
        let fixes = spell_corrections(
            &[
                "machien".into(),
                "lerning".into(),
                "interpretability".into(),
            ],
            &docs,
        );
        assert!(
            fixes.contains(&("machien".into(), "machine".into())),
            "{fixes:?}"
        );
        assert!(
            fixes.contains(&("lerning".into(), "learning".into())),
            "{fixes:?}"
        );
        assert!(!fixes.iter().any(|(a, _)| a == "interpretability"));
    }

    #[test]
    fn generic_words_never_make_a_doc_relevant() {
        let cands = vec![
            merged(
                "Heart attack treatment outcomes in older adults",
                None,
                "openalex",
                1,
            ),
            merged(
                "Treatment outcomes of insulin pens in diabetes",
                None,
                "openalex",
                2,
            ),
            merged("Heart attack registry", None, "openalex", 3),
        ];
        let ranked = rank_candidates(
            &q(&["heart", "attack", "treatment", "outcomes"]),
            cands,
            &HashMap::new(),
        );
        let kept: Vec<&str> = ranked
            .iter()
            .filter(|r| r.reject_reason.is_none())
            .map(|r| r.doc.doc.title.as_str())
            .collect();
        assert!(kept.contains(&"Heart attack treatment outcomes in older adults"));
        assert!(kept.contains(&"Heart attack registry"));
        assert!(!kept.iter().any(|t| t.contains("insulin")), "{kept:?}");
    }

    #[test]
    fn listing_allows_one_missing_concept_full_text_does_not() {
        let qm =
            q(&["sleep", "memory", "teenagers"]).with_expansion(&[("adolescents".into(), 0.5)]);
        let cands = vec![
            merged(
                "Sleep and memory consolidation in adolescents",
                None,
                "openalex",
                1,
            ),
            merged("Sleep and memory in mice", None, "openalex", 2),
            merged("Teenagers and sleep", None, "openalex", 3),
            merged("Memory and sleep in teenagers", None, "openalex", 5),
            merged("Sleep memory teenagers", None, "openalex", 4),
        ];
        let ranked = rank_candidates(&qm, cands, &HashMap::new());
        let kept: Vec<&str> = ranked
            .iter()
            .filter(|r| r.reject_reason.is_none())
            .map(|r| r.doc.doc.title.as_str())
            .collect();
        assert!(kept.contains(&"Sleep and memory consolidation in adolescents"));
        assert!(kept.contains(&"Sleep memory teenagers"));
        // The listing check lets "mice" through (2 of 3 concepts); the
        // full-text check is what rejects it.
        let pool: Vec<Vec<String>> = ["sleep memory teenager", "sleep memory adolescent"]
            .iter()
            .map(|t| stems(t))
            .collect();
        let c = concepts_for(&qm, &pool);
        assert_eq!(c.len(), 3, "{c:?}");
        let mice =
            "Sleep and memory in mice. Mice sleep; memory in mice improves after sleep.".repeat(5);
        assert!(!text_on_topic(&mice, &c));
        let teens = "Sleep and memory in adolescents. Teenagers who sleep more show better memory."
            .repeat(5);
        assert!(text_on_topic(&teens, &c));
    }

    #[test]
    fn full_text_topicality() {
        let qm = q(&["heart", "attack"]);
        let pool = vec![
            stems("heart attack outcomes"),
            stems("after a heart attack"),
            stems("heart attack registry"),
        ];
        let c = concepts_for(&qm, &pool);
        assert_eq!(c.len(), 1, "heart attack is one phrase: {c:?}");
        assert!(text_on_topic(
            "Heart attack care. After a heart attack, patients… heart attack recurrence.",
            &c
        ));
        assert!(text_on_topic(
            "Acute myocardial infarction: outcomes after myocardial infarction and recurrent myocardial infarction.",
            &c
        ));
        assert!(!text_on_topic(
            "A cyber attack on the heart of the grid. The attack vector and attack surface. The heart.",
            &c
        ));
        // Mentioned in passing inside a long unrelated paper: not on topic.
        let passing = format!(
            "{} We use heart attack data once. A heart attack example. {}",
            "Brownian motion hitting times are studied. ".repeat(400),
            "More stochastic calculus. ".repeat(400)
        );
        assert!(!text_on_topic(&passing, &c));
        assert!(metadata_on_topic("Heart attack in women", &c));
        assert!(!metadata_on_topic("Heart health", &c));
    }

    #[test]
    fn phrases_are_detected_from_the_pool_not_assumed() {
        let qm = q(&["roman", "empire", "economy"]);
        let pool = vec![
            stems("the economy of the roman empire"),
            stems("roman empire trade"),
            stems("roman empire coins and the economy"),
            stems("late roman empire"),
        ];
        let labels: Vec<String> = concepts_for(&qm, &pool)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["roman empire", "economy"]);
    }

    #[test]
    fn incidental_mentions_are_rejected() {
        let cands = vec![
            merged(
                "Parameter inference for perturbed Brownian motion",
                Some("We estimate hitting times and illustrate the method on heart attack data."),
                "arxiv",
                1,
            ),
            merged(
                "Outcomes after heart attack",
                Some("A registry study."),
                "openalex",
                1,
            ),
            merged(
                "Recovery in the first year",
                Some("Patients recovering from a heart attack; heart attack recurrence and heart attack care."),
                "openalex",
                2,
            ),
        ];
        let ranked = rank_candidates(&q(&["heart", "attack"]), cands, &HashMap::new());
        let kept: Vec<&str> = ranked
            .iter()
            .filter(|r| r.reject_reason.is_none())
            .map(|r| r.doc.doc.title.as_str())
            .collect();
        assert!(kept.contains(&"Outcomes after heart attack"));
        assert!(kept.contains(&"Recovery in the first year"));
        assert!(!kept.iter().any(|t| t.contains("Brownian")), "{kept:?}");
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
