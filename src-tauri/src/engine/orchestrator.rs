//! Core pipeline management for discovering, downloading, and bundling research documents.

use dashmap::DashSet;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::db::DbManager;
use super::downloader::{download, DownloadOutcome};
use super::extract::extract_text;
use super::query::{expand_query, parse_query, relevance_score, safe_folder};
use super::runlog;
use crate::events::*;
use crate::sources::{build_source, make_client, Document, SourceOptions, SOURCE_IDS};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunRequest {
    pub query: String,
    pub sources: Vec<String>,
    pub out_dir: String,
    #[serde(default = "default_per_source")]
    pub per_source: usize,
    #[serde(default = "default_max_total")]
    pub max_total: usize,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_extract")]
    pub extract: bool,
    #[serde(default)]
    pub source_options: HashMap<String, SourceOptions>,
}

fn default_per_source() -> usize {
    100
}
fn default_max_total() -> usize {
    500
}
fn default_concurrency() -> usize {
    8
}
fn default_extract() -> bool {
    true
}

pub async fn run_pipeline(
    app: AppHandle,
    req: RunRequest,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let client = Arc::new(make_client());
    let sub_queries = expand_query(&req.query);
    runlog::log(runlog::Event::RunStart {
        query: &req.query,
        sub_queries: &sub_queries,
        sources: &req.sources,
    });
    app.emit(
        EV_KEYWORDS,
        KeywordsPayload {
            query: req.query.clone(),
            sub_queries: sub_queries.clone(),
        },
    )?;

    let folder = PathBuf::from(&req.out_dir).join(safe_folder(&req.query));
    tokio::fs::create_dir_all(&folder).await?;
    let text_dir = folder.join("_text");
    if req.extract {
        tokio::fs::create_dir_all(&text_dir).await?;
    }

    let db_path = folder.join("library.db");
    let db_manager =
        Arc::new(DbManager::new(&db_path).map_err(|e| anyhow::anyhow!("DB init failed: {}", e))?);
    let run_id = db_manager
        .insert_run(&req.query, &folder.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("failed to insert run: {}", e))?;

    let seen_urls: DashSet<String> = DashSet::new();

    // -------- Phase 1: discovery (sequential per sub-query × source, but stream-driven) --------
    let mut candidates: Vec<Document> = Vec::new();
    'outer: for sub in &sub_queries {
        if cancel.is_cancelled() || candidates.len() >= req.max_total {
            break;
        }
        let keywords = parse_query(sub);
        let keywords = if keywords.is_empty() {
            sub.split_whitespace().map(String::from).collect::<Vec<_>>()
        } else {
            keywords
        };
        if keywords.is_empty() {
            continue;
        }
        app.emit(
            EV_SUBQUERY_START,
            SubQueryStartPayload {
                sub_query: sub.clone(),
                keywords: keywords.clone(),
            },
        )?;

        for sname in &req.sources {
            if cancel.is_cancelled() || candidates.len() >= req.max_total {
                break 'outer;
            }
            if !SOURCE_IDS.contains(&sname.as_str()) {
                let _ = app.emit(
                    EV_SOURCE_ERROR,
                    SourceErrorPayload {
                        source: sname.clone(),
                        error: "unknown source".into(),
                    },
                );
                continue;
            }
            let opts = req.source_options.get(sname).cloned().unwrap_or_default();
            let Some(src) = build_source(sname, opts, client.clone()) else {
                continue;
            };

            app.emit(
                EV_SOURCE_START,
                SourceStartPayload {
                    source: sname.clone(),
                    sub_query: sub.clone(),
                },
            )?;
            let mut count = 0usize;
            let mut filtered = 0usize;
            let mut stream = src.search(keywords.clone(), req.per_source).await;
            while let Some(item) = stream.next().await {
                if cancel.is_cancelled() || candidates.len() >= req.max_total {
                    break;
                }
                match item {
                    Ok(doc) => {
                        if !seen_urls.insert(doc.url.clone()) {
                            continue;
                        }
                        // Lexical relevance filter — drop documents that
                        // mention zero query keywords in title+abstract.
                        // Sources that don't return abstracts get judged on
                        // title alone, which is conservative.
                        let haystack =
                            format!("{} {}", doc.title, doc.abstract_.as_deref().unwrap_or(""));
                        if relevance_score(&keywords, &haystack) == 0 {
                            filtered += 1;
                            continue;
                        }
                        count += 1;
                        let _ = app.emit(
                            EV_FOUND,
                            FoundPayload {
                                title: doc.title.clone(),
                                source: doc.source.clone(),
                                url: doc.url.clone(),
                                total: candidates.len() + 1,
                            },
                        );
                        candidates.push(doc);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        runlog::log(runlog::Event::SourceError {
                            source: sname,
                            error: &err_str,
                            sub_query: Some(sub),
                        });
                        let _ = app.emit(
                            EV_SOURCE_ERROR,
                            SourceErrorPayload {
                                source: sname.clone(),
                                error: err_str,
                            },
                        );
                    }
                }
            }
            if filtered > 0 {
                let _ = app.emit(
                    EV_FILTERED,
                    FilteredPayload {
                        source: sname.clone(),
                        count: filtered,
                    },
                );
            }
            let _ = app.emit(
                EV_SOURCE_DONE,
                SourceDonePayload {
                    source: sname.clone(),
                    count,
                },
            );
        }
    }

    let _ = app.emit(
        EV_FOUND_TOTAL,
        FoundTotalPayload {
            count: candidates.len(),
        },
    );

    if candidates.is_empty() {
        let payload = CompletePayload {
            done: 0,
            failed: 0,
            total: 0,
            folder: folder.to_string_lossy().to_string(),
            manifest: manifest_path.to_string_lossy().to_string(),
        };
        let _ = app.emit(
            if cancel.is_cancelled() {
                EV_CANCELLED
            } else {
                EV_COMPLETE
            },
            payload,
        );
        return Ok(());
    }

    // -------- Phase 2: parallel downloads --------
    let total = candidates.len();
    let counters = Arc::new(tokio::sync::Mutex::new((0usize, 0usize))); // (done, failed)
    let semaphore = Arc::new(Semaphore::new(req.concurrency.max(1)));

    let mut handles = Vec::with_capacity(candidates.len());
    for doc in candidates {
        let app = app.clone();
        let client = client.clone();
        let cancel = cancel.clone();
        let semaphore = semaphore.clone();
        let counters = counters.clone();
        let db_manager = db_manager.clone();
        let folder = folder.clone();
        let text_dir = text_dir.clone();
        let extract_flag = req.extract;

        let handle = tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            if cancel.is_cancelled() {
                return;
            }

            let _ = app.emit(
                EV_DOWNLOAD_STARTED,
                DownloadStartedPayload {
                    url: doc.url.clone(),
                    title: doc.title.clone(),
                    source: doc.source.clone(),
                },
            );

            let app_for_progress = app.clone();
            let url_for_progress = doc.url.clone();
            let title_for_progress = doc.title.clone();
            let last_emit = std::sync::Mutex::new(std::time::Instant::now());

            let outcome = download(&doc, &folder, &client, &cancel, |ev| {
                // Throttle progress to ~5 updates/sec/file
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() < std::time::Duration::from_millis(200)
                    && (ev.total == 0 || ev.downloaded < ev.total)
                {
                    return;
                }
                *last = std::time::Instant::now();
                let _ = app_for_progress.emit(
                    EV_DOWNLOAD_PROGRESS,
                    DownloadProgressPayload {
                        url: url_for_progress.clone(),
                        title: title_for_progress.clone(),
                        downloaded: ev.downloaded,
                        total: ev.total,
                    },
                );
            })
            .await;

            match outcome {
                DownloadOutcome::Saved(path) => {
                    let bytes = tokio::fs::metadata(&path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    runlog::log(runlog::Event::DownloadOk {
                        source: &doc.source,
                        title: &doc.title,
                        url: &doc.url,
                        local_path: &path.to_string_lossy(),
                        bytes,
                    });
                    let mut text_path: Option<String> = None;
                    let mut extract_error: Option<String> = None;
                    if extract_flag {
                        let extract_path = path.clone();
                        let extract_res =
                            match tokio::task::spawn_blocking(move || extract_text(&extract_path))
                                .await
                            {
                                Ok(res) => res,
                                Err(e) => Err(anyhow::anyhow!("extraction task panicked: {}", e)),
                            };
                        match extract_res {
                            Ok(text) => {
                                if !text.trim().is_empty() {
                                    let stem = path
                                        .file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("doc")
                                        .to_string();
                                    let tpath = text_dir.join(format!("{stem}.txt"));
                                    if tokio::fs::write(&tpath, text).await.is_ok() {
                                        if let Ok(rel) = tpath.strip_prefix(&folder) {
                                            text_path = Some(rel.to_string_lossy().to_string());
                                        }
                                    }
                                } else {
                                    extract_error = Some("extracted text is empty".into());
                                }
                            }
                            Err(e) => {
                                let err_msg = e.to_string();
                                tracing::warn!(
                                    "extraction failed for {}: {}",
                                    path.display(),
                                    err_msg
                                );
                                extract_error = Some(err_msg);
                            }
                        }
                    }

                    let local_path = path
                        .strip_prefix(&folder)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());

                    // Persist to SQLite
                    let _ = db_manager.insert_document(
                        run_id,
                        &doc.title,
                        &doc.url,
                        &doc.source,
                        &doc.authors.join(", "),
                        doc.year.as_deref(),
                        doc.abstract_.as_deref(),
                        &local_path,
                        text_path.as_deref(),
                        extract_error.as_deref(),
                        bytes,
                    );

                    let (done, failed) = {
                        let mut c = counters.lock().await;
                        c.0 += 1;
                        (c.0, c.1)
                    };

                    let _ = app.emit(
                        EV_DOWNLOAD_DONE,
                        DownloadDonePayload {
                            doc,
                            local_path,
                            absolute_path: path.to_string_lossy().to_string(),
                            text_path,
                            done,
                            failed,
                            total,
                        },
                    );
                }
                DownloadOutcome::Failed(err) => {
                    runlog::log(runlog::Event::DownloadFail {
                        source: &doc.source,
                        title: &doc.title,
                        url: &doc.url,
                        error: &err,
                    });
                    let (done, failed) = {
                        let mut c = counters.lock().await;
                        c.1 += 1;
                        (c.0, c.1)
                    };
                    let _ = app.emit(
                        EV_DOWNLOAD_FAILED,
                        DownloadFailedPayload {
                            doc,
                            error: err,
                            done,
                            failed,
                            total,
                        },
                    );
                }
                DownloadOutcome::Cancelled => {
                    // Counted neither done nor failed; UI shows "cancelled" via cancel event.
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        if let Err(e) = h.await {
            if e.is_panic() {
                let msg = format!("download task panicked: {e}");
                tracing::error!("{msg}");
                let _ = app.emit(EV_ERROR, ErrorPayload { message: msg });
            }
        }
    }

    let (done, failed) = {
        let c = counters.lock().await;
        *c
    };

    let folder_str = folder.to_string_lossy().to_string();
    runlog::log(runlog::Event::RunComplete {
        done,
        failed,
        total,
        folder: &folder_str,
    });
    let payload = CompletePayload {
        done,
        failed,
        total,
        folder: folder_str,
        manifest: db_path.to_string_lossy().to_string(),
    };
    let _ = app.emit(
        if cancel.is_cancelled() {
            EV_CANCELLED
        } else {
            EV_COMPLETE
        },
        payload,
    );

    Ok(())
}
