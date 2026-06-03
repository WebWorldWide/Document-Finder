//! Out-of-process embedding worker.
//!
//! fastembed/ort can **abort natively** (SIGSEGV/SIGABRT) during ONNX Runtime
//! initialization on some platforms (observed on macOS). A native abort is not
//! a Rust panic — `catch_unwind` and the panic hook can't see it — so running
//! it in the UI process would take the whole app down. To contain it, ALL
//! fastembed/ort calls run here, in a child process that is just this same
//! binary re-invoked with [`WORKER_ARG`]. If the worker aborts, the parent
//! ([`super::embeddings`]) sees the pipe close and degrades to lexical ranking
//! instead of crashing.
//!
//! Protocol: one JSON [`WorkerRequest`] per line on stdin → one JSON
//! [`WorkerResponse`] per line on stdout. `phase:` markers go to stderr so the
//! parent can record where a native abort happened (the run log replaces the
//! macOS `.ips` report we otherwise can't get).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::embeddings::EmbeddingModel;

/// `argv[1]` sentinel that switches this binary into embedding-worker mode.
pub const WORKER_ARG: &str = "__embed-worker";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerRequest {
    /// Load (downloading on first use) the embedding model.
    Warm,
    /// Embed a batch of texts → one vector each.
    Embed { texts: Vec<String> },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ok", rename_all = "snake_case")]
pub enum WorkerResponse {
    Warmed,
    Embeddings {
        vectors: Vec<Vec<f32>>,
    },
    /// A *recoverable* failure (e.g. offline on first download). The worker
    /// stays alive so the user can retry. A native abort never produces this —
    /// the parent detects that as a closed pipe / abnormal exit.
    Error {
        message: String,
    },
}

/// Emit a phase marker to stderr. The parent keeps the last few lines, so if ort
/// aborts mid-phase the run log records exactly where.
fn phase(p: &str) {
    eprintln!("phase:{p}");
    let _ = std::io::stderr().flush();
}

fn ensure_loaded(model: &mut Option<EmbeddingModel>, cache_dir: &Path) -> anyhow::Result<()> {
    if model.is_none() {
        // `try_new` both downloads (first run) and builds the ONNX session —
        // the native abort we're isolating happens inside here.
        phase("load-start");
        let m = EmbeddingModel::init_with_cache_dir(cache_dir)?;
        phase("load-done");
        *model = Some(m);
    }
    Ok(())
}

/// Run the embedding worker loop until stdin closes, then exit. Never returns.
/// `cache_dir` (passed as `argv[2]`) is where fastembed caches the model.
pub fn run_worker(cache_dir: PathBuf) -> ! {
    phase("worker-start");
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut model: Option<EmbeddingModel> = None;

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break }; // stdin closed/broken → exit
        if line.trim().is_empty() {
            continue;
        }
        let req: WorkerRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                write_response(
                    &mut stdout,
                    &WorkerResponse::Error {
                        message: format!("bad request: {e}"),
                    },
                );
                continue;
            }
        };

        let resp = match req {
            WorkerRequest::Warm => match ensure_loaded(&mut model, &cache_dir) {
                Ok(()) => WorkerResponse::Warmed,
                Err(e) => WorkerResponse::Error {
                    message: format!("{e:#}"),
                },
            },
            WorkerRequest::Embed { texts } => {
                let result = ensure_loaded(&mut model, &cache_dir).and_then(|()| {
                    phase("embed-start");
                    let out = model
                        .as_mut()
                        .expect("model loaded by ensure_loaded")
                        .embed_batch(&texts);
                    phase("embed-done");
                    out
                });
                match result {
                    Ok(vectors) => WorkerResponse::Embeddings { vectors },
                    Err(e) => WorkerResponse::Error {
                        message: format!("{e:#}"),
                    },
                }
            }
        };
        write_response(&mut stdout, &resp);
    }

    std::process::exit(0);
}

fn write_response(stdout: &mut std::io::Stdout, resp: &WorkerResponse) {
    if let Ok(s) = serde_json::to_string(resp) {
        let _ = writeln!(stdout, "{s}");
        let _ = stdout.flush();
    }
}
