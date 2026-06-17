//! Real-network health probe for EVERY source + a download from each that
//! yields directly-downloadable results. `#[ignore]`d (hits live third-party
//! APIs); run on demand to find broken sources:
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml \
//!   --no-default-features --features=custom-protocol \
//!   --test source_health -- --ignored --nocapture
//! ```

use document_finder_lib::engine::downloader::{download, DownloadOutcome};
use document_finder_lib::sources::{
    build_source, make_client, make_download_client, Document, SourceOptions,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn search_source(
    id: &str,
    terms: &[&str],
    limit: usize,
    budget: Duration,
) -> (Vec<Document>, Option<String>) {
    let client = Arc::new(make_client());
    let Some(src) = build_source(id, SourceOptions::default(), client, None) else {
        return (vec![], Some("unknown source".into()));
    };
    let kw: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
    let mut docs = Vec::new();
    let mut err = None;
    let work = async {
        let mut stream = src.search(kw, limit).await;
        while let Some(item) = stream.next().await {
            match item {
                Ok(d) => {
                    docs.push(d);
                    if docs.len() >= limit {
                        break;
                    }
                }
                Err(e) => {
                    if err.is_none() {
                        err = Some(e.to_string());
                    }
                }
            }
        }
    };
    let _ = tokio::time::timeout(budget, work).await;
    (docs, err)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "hits the network; run with --ignored"]
async fn all_sources_search_health() {
    // Every default-enabled / selectable source id.
    let sources = [
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
    // Reported but not hard-asserted: semantic_scholar (keyless rate limits),
    // gutenberg (public-domain BOOKS — legitimately 0 for a modern topic; its
    // health is proven by the download test on a matching query), and
    // meta_search (the web scrapers depend on a residential IP — search engines
    // routinely block datacenter IPs, so a CI/sandbox run can see 0 even when it
    // works fine on a real user's machine).
    let flaky = ["semantic_scholar", "gutenberg", "meta_search"];

    let mut hard_failures = Vec::new();
    for id in sources {
        let (docs, err) =
            search_source(id, &["machine", "learning"], 8, Duration::from_secs(45)).await;
        println!(
            "SEARCH  {:<18} docs={:<3} err={}",
            id,
            docs.len(),
            err.as_deref().unwrap_or("-")
        );
        if docs.is_empty() && !flaky.contains(&id) {
            hard_failures.push(format!("{id}: 0 docs (err={err:?})"));
        }
    }
    assert!(
        hard_failures.is_empty(),
        "sources returning no results:\n{}",
        hard_failures.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "hits the network; run with --ignored"]
async fn downloadable_sources_actually_download() {
    // Sources whose top results are (or resolve to) direct documents.
    let sources = ["arxiv", "openalex", "doaj", "zenodo", "gutenberg"];
    let dl_client = make_download_client();
    let cancel = CancellationToken::new();

    let mut failures = Vec::new();
    for id in sources {
        let (docs, serr) =
            search_source(id, &["climate", "change"], 8, Duration::from_secs(45)).await;
        if docs.is_empty() {
            println!("DOWNLOAD {:<16} SKIP (no search results; err={serr:?})", id);
            // semantic_scholar-style flakiness aside, a downloadable source with
            // zero results is itself a problem.
            failures.push(format!("{id}: no search results to download"));
            continue;
        }
        // Try up to 4 of the source's results; count it a pass if ANY saves
        // (some individual links are dead/paywalled even for healthy sources).
        let dir = tempfile::tempdir().unwrap();
        let mut saved = false;
        let mut last_err = String::new();
        for doc in docs.iter().take(4) {
            match download(doc, dir.path(), &dl_client, &cancel, |_| {}).await {
                DownloadOutcome::Saved(_) | DownloadOutcome::Cached(_) => {
                    saved = true;
                    break;
                }
                DownloadOutcome::Failed(e) => last_err = e,
                DownloadOutcome::Cancelled => {}
            }
        }
        println!(
            "DOWNLOAD {:<16} {}",
            id,
            if saved {
                "OK".to_string()
            } else {
                format!("FAILED (last: {last_err})")
            }
        );
        if !saved {
            failures.push(format!(
                "{id}: no downloadable result in top 4 (last err: {last_err})"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "download failures:\n{}",
        failures.join("\n")
    );
}
