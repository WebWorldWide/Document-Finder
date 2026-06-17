//! Real-network smoke test for discovery + download.
//!
//! These hit live third-party APIs, so they are `#[ignore]`d — CI runs plain
//! `cargo test` (which skips them) and a developer runs them on demand:
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml \
//!   --no-default-features --features=custom-protocol \
//!   --test network_smoke -- --ignored --nocapture
//! ```
//!
//! They verify the user-reported bugs are actually fixed end-to-end:
//! a source returns documents quickly, and a landing-page URL downloads to a
//! valid PDF on disk.

use document_finder_lib::engine::downloader::{download, DownloadOutcome};
use document_finder_lib::sources::{
    build_source, make_client, make_download_client, Document, Source, SourceOptions,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Collect up to `want` documents from a source, bounded by `budget`.
async fn collect_some(
    src: &dyn Source,
    terms: &[&str],
    want: usize,
    budget: Duration,
) -> Vec<Document> {
    let keywords: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
    let mut docs = Vec::new();
    let work = async {
        let mut stream = src.search(keywords, 25).await;
        while let Some(item) = stream.next().await {
            if let Ok(d) = item {
                docs.push(d);
                if docs.len() >= want {
                    break;
                }
            }
        }
    };
    let _ = tokio::time::timeout(budget, work).await;
    docs
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "hits the network; run with --ignored"]
async fn arxiv_search_returns_documents_quickly() {
    let client = Arc::new(make_client());
    let src = build_source("arxiv", SourceOptions::default(), client, None).expect("arxiv source");

    let t = Instant::now();
    let docs = collect_some(
        src.as_ref(),
        &["transformer", "attention"],
        3,
        Duration::from_secs(40),
    )
    .await;
    let elapsed = t.elapsed();

    println!("arxiv: {} docs in {:?}", docs.len(), elapsed);
    assert!(!docs.is_empty(), "arXiv returned no documents");
    assert!(
        docs.iter()
            .all(|d| !d.url.is_empty() && !d.title.is_empty()),
        "arXiv docs missing url/title"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "hits the network; run with --ignored"]
async fn openalex_search_returns_documents() {
    let client = Arc::new(make_client());
    let src =
        build_source("openalex", SourceOptions::default(), client, None).expect("openalex source");

    let t = Instant::now();
    let docs = collect_some(
        src.as_ref(),
        &["machine", "learning"],
        3,
        Duration::from_secs(40),
    )
    .await;
    println!("openalex: {} docs in {:?}", docs.len(), t.elapsed());
    assert!(!docs.is_empty(), "OpenAlex returned no documents");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "hits the network; run with --ignored"]
async fn downloads_a_real_pdf_from_a_landing_page_url() {
    // An arXiv /abs/ landing page — canonicalize_doc_url rewrites it to /pdf/,
    // which serves the PDF directly. Exercises the real download path.
    let doc = Document {
        title: "Attention Is All You Need".into(),
        url: "https://arxiv.org/abs/1706.03762".into(),
        source: "arxiv".into(),
        authors: vec![],
        year: None,
        abstract_: None,
        identifier: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let client = make_download_client();
    let cancel = CancellationToken::new();

    let t = Instant::now();
    let outcome = download(&doc, dir.path(), &client, &cancel, |_| {}).await;
    println!("download finished in {:?}", t.elapsed());

    let path = match outcome {
        DownloadOutcome::Saved(p) | DownloadOutcome::Cached(p) => p,
        DownloadOutcome::Failed(e) => panic!("download failed: {e}"),
        DownloadOutcome::Cancelled => panic!("download unexpectedly cancelled"),
    };
    let bytes = std::fs::read(&path).expect("read saved file");
    println!("saved {} ({} bytes)", path.display(), bytes.len());
    assert!(bytes.len() > 4096, "saved file is suspiciously small");
    assert!(
        bytes.windows(4).any(|w| w == b"%PDF"),
        "saved file is not a valid PDF"
    );
}
