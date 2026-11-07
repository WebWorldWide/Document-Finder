//! Headless pipeline benchmark against the live sources.
//!
//!   cargo run --release --example bench -- "query one" "query two" …
//!
//! For each query it runs the full search → download → extract pipeline into
//! a temp library and reports time-to-first-saved-doc, saved / failed counts,
//! throughput, the top failure reasons, and the share of saved documents
//! with extracted text. With no arguments it runs a fixed 12-query suite.
use document_finder_lib::engine::{run_pipeline, RunRequest};
use document_finder_lib::events::Sink;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const SUITE: &[&str] = &[
    "transformer attention mechanism",
    "climate change agriculture africa",
    "heart attack treatment outcomes",
    "CRISPR gene editing ethics",
    "roman empire economy",
    "graph neural networks drug discovery",
    "microplastics ocean pollution",
    "machien lerning interpretability",
    "victorian novels",
    "covid-19 vaccine efficacy",
    "quantum error correction",
    "urban heat island mitigation",
];

const SOURCES: &[&str] = &[
    "arxiv",
    "openalex",
    "semantic_scholar",
    "europe_pmc",
    "internet_archive",
    "doaj",
    "zenodo",
    "gutenberg",
    "meta_search",
];

#[derive(Default)]
struct Stats {
    t0: Option<Instant>,
    first_saved: Option<Duration>,
    found: usize,
    saved: usize,
    failed: usize,
    with_text: usize,
    bytes: u64,
    reasons: HashMap<String, usize>,
    titles: Vec<String>,
    done: bool,
}

struct Rec(Mutex<Stats>);

impl Sink for Rec {
    fn emit_value(&self, event: &str, p: serde_json::Value) {
        let mut s = self.0.lock();
        match event {
            "df:found" => s.found = p["total"].as_u64().unwrap_or(0) as usize,
            "df:download_done" => {
                s.saved += 1;
                s.titles.push(p["title"].as_str().unwrap_or("").to_string());
                if s.first_saved.is_none() {
                    s.first_saved = s.t0.map(|t| t.elapsed());
                }
                if !p["text_path"].is_null() {
                    s.with_text += 1;
                }
                s.bytes += p["bytes"].as_u64().unwrap_or(0);
            }
            "df:download_failed" => {
                s.failed += 1;
                let r = p["error"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(60)
                    .collect::<String>();
                *s.reasons.entry(r).or_insert(0) += 1;
            }
            "df:candidate" => {
                let key = match p["reject_reason"].as_str() {
                    Some(r) => format!("(rejected) {r}"),
                    None => "(kept)".into(),
                };
                *s.reasons.entry(key).or_insert(0) += 1;
            }
            "df:download_skipped" => {
                *s.reasons.entry("(skipped quietly)".into()).or_insert(0) += 1;
            }
            "df:complete" | "df:cancelled" => s.done = true,
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let queries: Vec<String> = if args.is_empty() {
        SUITE.iter().map(|s| s.to_string()).collect()
    } else {
        args
    };
    println!("query\tfirst_saved_s\tfound\tsaved\tfailed\ttext%\tMB\twall_s");
    for q in queries {
        let dir = tempfile::tempdir().unwrap();
        let rec = Arc::new(Rec(Mutex::new(Stats {
            t0: Some(Instant::now()),
            ..Default::default()
        })));
        let sink: Arc<dyn Sink> = rec.clone();
        let t0 = Instant::now();
        let req = RunRequest {
            query: q.clone(),
            sources: SOURCES.iter().map(|s| s.to_string()).collect(),
            out_dir: dir.path().to_string_lossy().to_string(),
            source_options: Default::default(),
        };
        if let Err(e) = run_pipeline(sink, None, req, CancellationToken::new()).await {
            eprintln!("{q}: error {e}");
            continue;
        }
        let s = rec.0.lock();
        println!(
            "{q}\t{:.1}\t{}\t{}\t{}\t{:.0}\t{:.1}\t{:.1}",
            s.first_saved.map(|d| d.as_secs_f32()).unwrap_or(f32::NAN),
            s.found,
            s.saved,
            s.failed,
            if s.saved > 0 {
                100.0 * s.with_text as f32 / s.saved as f32
            } else {
                0.0
            },
            s.bytes as f64 / 1e6,
            t0.elapsed().as_secs_f32()
        );
        if let Ok(dir) = std::env::var("BENCH_TITLES") {
            let f = format!("{dir}/{}.txt", q.replace(' ', "_"));
            let _ = std::fs::write(&f, s.titles.join("\n"));
        }
        let mut reasons: Vec<_> = s.reasons.iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(a.1));
        for (r, n) in reasons.iter().take(8) {
            eprintln!("    {n:>3} × {r}");
        }
    }
}
