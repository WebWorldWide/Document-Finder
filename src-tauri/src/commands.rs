//! Tauri commands invoked from the Solid.js frontend.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::ai;
use crate::ai::registry;
use crate::ai::state::{snapshot, AiState, ModelInfo, ModelStatus};
use crate::engine::db::init_db;
use crate::engine::runlog;
use crate::engine::{run_pipeline, RunRequest};
use crate::events::{
    ErrorPayload, SearxngLogPayload, SearxngStagePayload, EV_ERROR, EV_SEARXNG_LOG,
    EV_SEARXNG_STAGE,
};
use crate::sources::USER_AGENT;

/// HTTP client tuned for huge model downloads — connect_timeout fails fast on
/// dead URLs but there is NO overall .timeout, because the default
/// `make_client()` uses 60s which would silently kill a 2GB download
/// mid-stream.
fn make_model_download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(30))
        // Intentionally NO .timeout — model files take many minutes.
        .build()
        .expect("model http client")
}

#[derive(Default)]
pub struct AppState {
    /// Active run's cancellation token, if any.
    pub current_cancel: Mutex<Option<CancellationToken>>,
}

#[derive(Debug, Serialize)]
pub struct DefaultDirsResp {
    pub library_root: String,
}

/// Returns the default library root, e.g. ~/Documents/Document Finder/library.
#[tauri::command]
pub fn default_library_dir() -> Result<DefaultDirsResp, String> {
    let docs = dirs::document_dir().ok_or("could not find Documents directory")?;
    let lib = docs.join("Document Finder").join("library");
    std::fs::create_dir_all(&lib).map_err(|e| e.to_string())?;
    Ok(DefaultDirsResp {
        library_root: lib.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    req: RunRequest,
) -> Result<(), String> {
    {
        let mut cur = state
            .current_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cur.is_some() {
            return Err("a run is already in progress".to_string());
        }
        let token = CancellationToken::new();
        *cur = Some(token.clone());
        drop(cur);
        let app2 = app.clone();
        tokio::spawn(async move {
            use tauri::Emitter;
            let result = run_pipeline(app2.clone(), req, token).await;
            // Clear token regardless of outcome.
            if let Some(state) = app2.try_state::<AppState>() {
                let mut cur = state
                    .current_cancel
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *cur = None;
            }
            if let Err(e) = result {
                let _ = app2.emit(EV_ERROR, ErrorPayload { message: e.to_string() });
            }
        });
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_run(state: State<'_, AppState>) -> Result<(), String> {
    let cur = state
        .current_cancel
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(token) = &*cur {
        token.cancel();
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct LibraryInfo {
    pub name: String,
    pub path: String,
    pub query: String,
    pub n_docs: usize,
    pub size_bytes: u64,
}

fn folder_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        // Use symlink_metadata so we never follow symlinks (avoids cycles / traversal).
        let Ok(meta) = entry.path().symlink_metadata() else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += folder_size_bytes(&entry.path());
        }
    }
    total
}

#[tauri::command]
pub async fn list_libraries(root: String) -> Result<Vec<LibraryInfo>, String> {
    let root = PathBuf::from(root);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let db_path = path.join("library.db");
        let manifest_path = path.join("manifest.json");

        if !db_path.exists() && manifest_path.exists() {
            // Migration logic: JSON to SQLite
            if let Ok(raw) = std::fs::read(&manifest_path) {
                if let Ok(m) = serde_json::from_slice::<crate::engine::manifest::Manifest>(&raw) {
                    if let Ok(mgr) = crate::engine::db::DbManager::new(&db_path) {
                        if let Ok(run_id) = mgr.insert_run(&m.query, &path.to_string_lossy()) {
                            for e in m.documents {
                                let _ = mgr.insert_document(
                                    run_id,
                                    &e.doc.title,
                                    &e.doc.url,
                                    &e.doc.source,
                                    &e.doc.authors.join(", "),
                                    e.doc.year.as_deref(),
                                    e.doc.abstract_.as_deref(),
                                    &e.local_path,
                                    e.text_path.as_deref(),
                                    e.extract_error.as_deref(),
                                    0, // size unknown
                                );
                            }
                        }
                    }
                }
            }
        }

        if !db_path.exists() {
            continue;
        }

        let path_clone = path.clone();
        let info = tokio::task::spawn_blocking(move || -> Result<Option<LibraryInfo>, String> {
            let conn = init_db(&db_path).map_err(|e| e.to_string())?;
            let row = conn
                .query_row(
                    "SELECT r.query, (SELECT COUNT(*) FROM documents WHERE run_id = r.id)
                 FROM runs r ORDER BY r.created_at DESC LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;

            let Some((query, n_docs)) = row else {
                return Ok(None); // empty db — skip
            };

            Ok(Some(LibraryInfo {
                name: path_clone
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path_clone.to_string_lossy().to_string(),
                query,
                n_docs,
                size_bytes: folder_size_bytes(&path_clone),
            }))
        })
        .await
        .map_err(|e| e.to_string())??;
        if let Some(info) = info {
            out.push(info);
        }
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

#[tauri::command]
pub async fn open_library(path: String) -> Result<LibraryInfo, String> {
    let p = PathBuf::from(&path);
    let db_path = p.join("library.db");
    if !db_path.exists() {
        return Err("library.db not found".into());
    }

    let p_clone = p.clone();
    let info = tokio::task::spawn_blocking(move || -> Result<LibraryInfo, String> {
        let conn = init_db(&db_path).map_err(|e| e.to_string())?;
        let row = conn
            .query_row(
                "SELECT r.query, (SELECT COUNT(*) FROM documents WHERE run_id = r.id)
             FROM runs r ORDER BY r.created_at DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let (query, n_docs) = row.unwrap_or_else(|| ("(no runs)".to_string(), 0));

        Ok(LibraryInfo {
            name: p_clone
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: p_clone.to_string_lossy().to_string(),
            query,
            n_docs,
            size_bytes: folder_size_bytes(&p_clone),
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(info)
}

#[derive(Debug, Deserialize)]
pub struct ExportArgs {
    pub folder: String,
    pub dest: String,
    /// When true, also include extracted text (`_text/`). Default true so the
    /// resulting ZIP works with AIs that prefer plain text.
    #[serde(default = "default_true")]
    pub include_text: bool,
    /// When true, include original PDFs/EPUBs/HTML. Default true.
    #[serde(default = "default_true")]
    pub include_originals: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub dest: String,
    pub files: usize,
    pub size_bytes: u64,
}

/// Pack a library folder as a single ZIP that's easy to upload to AI tools
/// (Claude Projects, ChatGPT, etc.). Skips the BM25 index and OS junk.
#[tauri::command]
pub fn export_library_zip(args: ExportArgs) -> Result<ExportResult, String> {
    let src = PathBuf::from(&args.folder)
        .canonicalize()
        .map_err(|e| format!("invalid folder: {}", e))?;
    if !src.is_dir() {
        return Err(format!("not a folder: {}", args.folder));
    }
    // Confirm the folder is inside the user's Documents directory.
    let docs = dirs::document_dir().ok_or("cannot resolve Documents directory")?;
    if !src.starts_with(&docs) {
        return Err("folder must be inside your Documents directory".to_string());
    }
    let dest_path = PathBuf::from(&args.dest);
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(&dest_path).map_err(|e| e.to_string())?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    let root_name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "library".to_string());

    let mut count = 0usize;
    write_zip_recursive(
        &mut zip_writer,
        &src,
        &PathBuf::from(&root_name),
        &options,
        &mut count,
        &args,
    )
    .map_err(|e| e.to_string())?;
    zip_writer.finish().map_err(|e| e.to_string())?;

    let size_bytes = std::fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
    Ok(ExportResult {
        dest: dest_path.to_string_lossy().to_string(),
        files: count,
        size_bytes,
    })
}

fn write_zip_recursive(
    zw: &mut zip::ZipWriter<std::fs::File>,
    src: &Path,
    rel: &Path,
    options: &zip::write::FileOptions<()>,
    count: &mut usize,
    args: &ExportArgs,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        // Skip OS junk and the (now-removed) BM25 index dir if it exists from older runs.
        if name_str == ".DS_Store" || name_str == "_index" || name_str.starts_with('.') {
            continue;
        }

        // Never follow symlinks — prevents traversal out of the library folder.
        let sym_meta = std::fs::symlink_metadata(&path)?;
        if sym_meta.file_type().is_symlink() {
            continue;
        }

        let new_rel = rel.join(&name);
        if path.is_dir() {
            // Skip _text dir if user opted out.
            if name_str == "_text" && !args.include_text {
                continue;
            }
            write_zip_recursive(zw, &path, &new_rel, options, count, args)?;
        } else {
            // Skip original docs if user opted out — but always include manifest.json.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let is_doc = matches!(ext.as_str(), "pdf" | "epub" | "html" | "htm");
            if is_doc && !args.include_originals {
                continue;
            }

            zw.start_file(new_rel.to_string_lossy().as_ref(), *options)?;
            let mut f = std::fs::File::open(&path)?;
            std::io::copy(&mut f, zw)?;
            *count += 1;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct LogInfo {
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
}

#[tauri::command]
pub fn run_log_info() -> Result<LogInfo, String> {
    let path = runlog::log_path().ok_or("could not resolve log directory")?;
    let (exists, size_bytes) = match std::fs::metadata(&path) {
        Ok(m) => (true, m.len()),
        Err(_) => (false, 0),
    };
    Ok(LogInfo {
        path: path.to_string_lossy().to_string(),
        exists,
        size_bytes,
    })
}

#[tauri::command]
pub fn run_log_tail(max: Option<usize>) -> Result<Vec<serde_json::Value>, String> {
    Ok(runlog::read_tail(max.unwrap_or(200)))
}

/// Container name we own. Anything with this name gets removed on setup;
/// users running their own SearXNG should use a different name.
const SEARXNG_CONTAINER: &str = "document-finder-searxng";

/// Pick a host port for SearXNG. Tries 8080 first; if it's bound, walks up
/// from 8888 until one is free. Returns the first port that `bind` accepts.
async fn find_free_port() -> Result<u16, String> {
    use tokio::net::TcpListener;
    let candidates: Vec<u16> = std::iter::once(8080)
        .chain(8888..=8910)
        .collect();
    for port in candidates {
        if TcpListener::bind(("127.0.0.1", port)).await.is_ok() {
            return Ok(port);
        }
    }
    Err("no free local port found in 8080, 8888..=8910".into())
}

fn emit_log(app: &AppHandle, stream: &str, line: impl Into<String>) {
    let _ = app.emit(
        EV_SEARXNG_LOG,
        SearxngLogPayload {
            stream: stream.to_string(),
            line: line.into(),
        },
    );
}

fn emit_stage(app: &AppHandle, stage: &str, detail: Option<String>) {
    let _ = app.emit(
        EV_SEARXNG_STAGE,
        SearxngStagePayload {
            stage: stage.to_string(),
            detail,
        },
    );
}

/// Spawn `docker <args>` and stream stdout+stderr line-by-line to the frontend.
/// Returns Ok(exit_code) regardless of success — caller decides what's fatal.
async fn run_docker_streaming(
    app: &AppHandle,
    args: &[&str],
) -> Result<std::process::ExitStatus, String> {
    emit_log(app, "info", format!("$ docker {}", args.join(" ")));

    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn docker: {}", e))?;

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;

    let app_out = app.clone();
    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // docker pull uses `\r` for in-place progress; the `lines` iterator
            // would treat them as a single huge line. Split on `\r` to get one
            // event per progress update.
            for chunk in line.split('\r') {
                let trimmed = chunk.trim();
                if !trimmed.is_empty() {
                    emit_log(&app_out, "stdout", trimmed.to_string());
                }
            }
        }
    });

    let app_err = app.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            for chunk in line.split('\r') {
                let trimmed = chunk.trim();
                if !trimmed.is_empty() {
                    emit_log(&app_err, "stderr", trimmed.to_string());
                }
            }
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("docker wait failed: {}", e))?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    Ok(status)
}

/// Verify SearXNG is actually serving JSON results — not just returning 200
/// for the static home page. Default SearXNG ships with JSON disabled, so a
/// container started without our settings.yml will pass a naive HTTP probe
/// but reject every real query with 403. This is the silent failure mode
/// the original `setup_searxng` exhibited.
async fn searxng_json_health(http: &reqwest::Client, base_url: &str) -> Result<(), String> {
    let url = format!("{}/search", base_url);
    let resp = http
        .get(&url)
        .query(&[
            ("q", "test"),
            ("format", "json"),
            ("categories", "general"),
        ])
        .send()
        .await
        .map_err(|e| format!("network error: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "GET {}?format=json returned HTTP {} — JSON output is likely disabled in settings.yml",
            url, status
        ));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("non-JSON response (bot-protection or limiter active?): {}", e))?;
    if parsed.get("results").is_none() {
        return Err("JSON response missing `results` key".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn setup_searxng(app: AppHandle) -> Result<String, String> {
    // ---- Stage 1: Docker available? ---------------------------------------
    emit_stage(&app, "checking_docker", None);
    let docker_info = tokio::process::Command::new("docker")
        .arg("info")
        .output()
        .await
        .map_err(|_| {
            emit_stage(&app, "failed", Some("docker not installed".into()));
            "Docker is not installed. Install Docker Desktop from https://www.docker.com/products/docker-desktop/".to_string()
        })?;
    if !docker_info.status.success() {
        emit_stage(&app, "failed", Some("docker daemon not running".into()));
        return Err("Docker is not running. Please start Docker Desktop and try again.".into());
    }

    // ---- Stage 2: Resolve mounted settings.yml ----------------------------
    let settings_path = app
        .path()
        .resolve("resources/searxng-settings.yml", BaseDirectory::Resource)
        .map_err(|e| {
            emit_stage(&app, "failed", Some(format!("resource resolve: {}", e)));
            format!("could not locate bundled searxng-settings.yml: {}", e)
        })?;
    if !settings_path.exists() {
        emit_stage(&app, "failed", Some("settings.yml missing from bundle".into()));
        return Err(format!(
            "bundled searxng-settings.yml not found at {}",
            settings_path.display()
        ));
    }
    let settings_path_str = settings_path.to_string_lossy().to_string();
    emit_log(&app, "info", format!("config: {}", settings_path_str));

    // ---- Stage 3: Pick free host port -------------------------------------
    emit_stage(&app, "checking_port", None);
    let host_port = find_free_port().await?;
    emit_log(&app, "info", format!("host port: {}", host_port));

    // ---- Stage 4: Pull image ----------------------------------------------
    emit_stage(&app, "pulling", None);
    let pull_status = run_docker_streaming(&app, &["pull", "searxng/searxng:latest"]).await?;
    if !pull_status.success() {
        emit_stage(&app, "failed", Some("docker pull failed".into()));
        return Err("docker pull failed — see modal log for details".into());
    }

    // ---- Stage 5: Remove any stale container ------------------------------
    let _ = run_docker_streaming(&app, &["rm", "-f", SEARXNG_CONTAINER]).await;

    // ---- Stage 6: Start container with mounted settings.yml ---------------
    emit_stage(&app, "starting", Some(format!("port {}", host_port)));
    let port_arg = format!("{}:8080", host_port);
    let mount_arg = format!("{}:/etc/searxng/settings.yml:ro", settings_path_str);
    let run_status = run_docker_streaming(
        &app,
        &[
            "run",
            "-d",
            "--name",
            SEARXNG_CONTAINER,
            "--restart",
            "unless-stopped",
            "-p",
            &port_arg,
            "-v",
            &mount_arg,
            "searxng/searxng:latest",
        ],
    )
    .await?;
    if !run_status.success() {
        emit_stage(&app, "failed", Some("docker run failed".into()));
        return Err("docker run failed — see modal log for details".into());
    }

    // ---- Stage 7: Wait for JSON-capable health ----------------------------
    emit_stage(&app, "waiting_health", None);
    let base_url = format!("http://localhost:{}", host_port);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .unwrap_or_default();

    let mut last_err = String::from("never reached");
    for attempt in 1..=20 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        match searxng_json_health(&http, &base_url).await {
            Ok(()) => {
                emit_stage(&app, "ok", Some(base_url.clone()));
                let msg = format!(
                    "SearXNG is healthy and serving JSON.\nSEARXNG_URL={}",
                    base_url
                );
                emit_log(&app, "info", msg.clone());
                return Ok(msg);
            }
            Err(e) => {
                last_err = e.clone();
                emit_log(
                    &app,
                    "info",
                    format!("health check {}: {}", attempt, e),
                );
            }
        }
    }

    emit_stage(&app, "failed", Some(last_err.clone()));
    Err(format!(
        "SearXNG container is up but JSON health check never passed.\nLast error: {}\nThe container is still running on {} — you can `docker logs {}` for details.",
        last_err, base_url, SEARXNG_CONTAINER
    ))
}

#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    // Reject relative paths and URI-scheme strings (e.g. "file:///etc").
    if !p.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    if path.contains("://") {
        return Err("path must not be a URI".to_string());
    }
    if !p.exists() {
        return Err(format!("not found: {}", path));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // Pass /select and the path as two separate args so commas in the
        // path cannot split the argument and confuse Explorer's parser.
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let parent = p.parent().unwrap_or(std::path::Path::new("/"));
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

// =============================================================================
// AI Model Manager commands (E1)
// =============================================================================

#[tauri::command]
pub fn list_models(app: AppHandle, state: State<'_, AiState>) -> Result<Vec<ModelInfo>, String> {
    // Wrap in catch_unwind so a panic anywhere in `from_entry` (mutex poison,
    // path resolution, registry access) becomes a real error string the UI
    // can show instead of a hung promise.
    //
    // AppHandle and State refs aren't UnwindSafe (they hold Arcs into mutex
    // chains); AssertUnwindSafe is appropriate because if a panic does occur
    // we're going to surface and bail, not continue using the borrowed state.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        snapshot(&app, &state)
    }));
    match result {
        Ok(list) => {
            tracing::info!("list_models: returning {} entries", list.len());
            Ok(list)
        }
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            tracing::error!("list_models panicked: {}", detail);
            Err(format!("list_models panicked: {}", detail))
        }
    }
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, AiState>,
    model_id: String,
) -> Result<(), String> {
    let entry = registry::find(&model_id)
        .ok_or_else(|| format!("unknown model id: {}", model_id))?;

    if state.is_downloading(&model_id) {
        return Err(format!("model {} is already downloading", model_id));
    }

    let token = tokio_util::sync::CancellationToken::new();
    state.register_cancel(&model_id, token.clone());
    state.set_status(
        &model_id,
        ModelStatus::Downloading {
            downloaded: 0,
            total: entry.approx_bytes,
        },
    );

    // Use the dedicated no-timeout client; the shared make_client() has a
    // 60s overall timeout that silently kills 2 GB downloads mid-stream.
    let client = std::sync::Arc::new(make_model_download_client());
    let app_for_task = app.clone();
    let model_id_for_task = model_id.clone();

    // Spawn so the command returns immediately; progress streams via events.
    tokio::spawn(async move {
        let result = ai::downloader::download(app_for_task.clone(), client, entry, token).await;
        if let Some(state) = app_for_task.try_state::<AiState>() {
            state.clear_cancel(&model_id_for_task);
            match &result {
                Ok(_) => state.set_status(&model_id_for_task, ModelStatus::Ready),
                Err(e) => {
                    let msg = e.to_string();
                    if msg == "cancelled" {
                        state.set_status(&model_id_for_task, ModelStatus::Cancelled);
                    } else {
                        state.set_status(&model_id_for_task, ModelStatus::Failed { msg });
                    }
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn cancel_model_download(state: State<'_, AiState>, model_id: String) -> Result<(), String> {
    state.cancel_download(&model_id);
    Ok(())
}

#[tauri::command]
pub async fn delete_model(
    app: AppHandle,
    state: State<'_, AiState>,
    model_id: String,
) -> Result<(), String> {
    let entry = registry::find(&model_id)
        .ok_or_else(|| format!("unknown model id: {}", model_id))?;

    // Cancel any in-flight download first.
    state.cancel_download(&model_id);

    let dir = ai::storage::model_dir(&app, entry).map_err(|e| e.to_string())?;
    if dir.exists() {
        let stage = force_remove_dir(&dir)
            .await
            .map_err(|e| format!("failed to delete {}: {}", dir.display(), e))?;
        tracing::info!("delete_model: removed {} via {}", dir.display(), stage);
    }
    state.set_status(&model_id, ModelStatus::NotDownloaded);
    Ok(())
}

// =============================================================================
// ACL-proof directory removal
// =============================================================================

/// Recursively delete a directory, working around macOS Extended ACLs and
/// quarantine xattrs that block `unlink` even when the user owns the file.
///
/// On macOS, `chmod +a` ACLs (or those inherited from a parent dir) cause
/// `tokio::fs::remove_dir_all` to fail with `PermissionDenied` — *not*
/// because anything has the file open, but because the ACL denies
/// `delete`. Same with the `com.apple.quarantine` xattr after a download.
///
/// The fallback chain:
///   1. plain `remove_dir_all` (the normal path)
///   2. `chmod -RN` to strip ACLs, then `xattr -rc` to clear xattrs, then retry
///   3. last-resort shell `rm -rf` (path is already validated by the caller
///      to live inside Documents/the app's controlled tree)
///
/// On success, returns the stage name that succeeded so the caller can log
/// it. On failure, returns the chain of error messages.
async fn force_remove_dir(path: &Path) -> Result<&'static str, String> {
    // Stage 1: the easy path.
    if let Err(e) = tokio::fs::remove_dir_all(path).await {
        let stage1_msg = format!("remove_dir_all: {} ({:?})", e, e.kind());
        tracing::info!(
            "force_remove_dir stage 1 failed for {}: {}",
            path.display(),
            stage1_msg
        );

        // If the directory is already gone, treat as success — this can
        // happen if a previous attempt half-succeeded.
        if e.kind() == std::io::ErrorKind::NotFound {
            return Ok("already gone");
        }

        // Stage 2: strip macOS ACLs + xattrs, then retry.
        #[cfg(target_os = "macos")]
        {
            let chmod_status = tokio::process::Command::new("chmod")
                .arg("-RN")
                .arg(path)
                .output()
                .await;
            let xattr_status = tokio::process::Command::new("xattr")
                .arg("-rc")
                .arg(path)
                .output()
                .await;
            let chmod_ok = chmod_status
                .as_ref()
                .map(|o| o.status.success())
                .unwrap_or(false);
            let xattr_ok = xattr_status
                .as_ref()
                .map(|o| o.status.success())
                .unwrap_or(false);
            tracing::info!(
                "force_remove_dir stage 2 (chmod -RN ok={}, xattr -rc ok={}) for {}",
                chmod_ok,
                xattr_ok,
                path.display()
            );

            if let Err(e2) = tokio::fs::remove_dir_all(path).await {
                let stage2_msg = format!("after-strip remove_dir_all: {}", e2);
                tracing::info!(
                    "force_remove_dir stage 2 failed for {}: {}",
                    path.display(),
                    stage2_msg
                );

                // Stage 3: shell `rm -rf` as a last resort.
                let rm_out = tokio::process::Command::new("rm")
                    .arg("-rf")
                    .arg(path)
                    .output()
                    .await
                    .map_err(|spawn_err| {
                        format!(
                            "all delete strategies failed; final stage couldn't even spawn rm: {} (stage 1: {}, stage 2: {})",
                            spawn_err, stage1_msg, stage2_msg
                        )
                    })?;
                if !rm_out.status.success() {
                    let stderr = String::from_utf8_lossy(&rm_out.stderr).into_owned();
                    return Err(format!(
                        "all delete strategies failed.\n  stage 1 (remove_dir_all): {}\n  stage 2 (after chmod -RN + xattr -rc): {}\n  stage 3 (rm -rf): {}",
                        stage1_msg, stage2_msg, stderr.trim()
                    ));
                }
                tracing::info!(
                    "force_remove_dir stage 3 (rm -rf) succeeded for {}",
                    path.display()
                );
                return Ok("rm -rf");
            }
            tracing::info!(
                "force_remove_dir stage 2 (post chmod/xattr) succeeded for {}",
                path.display()
            );
            return Ok("chmod -RN + xattr -rc");
        }

        // Non-macOS: no ACL stripping toolchain assumed; just propagate.
        #[cfg(not(target_os = "macos"))]
        {
            return Err(stage1_msg);
        }
    }
    Ok("remove_dir_all")
}

// =============================================================================
// Library delete (F3)
// =============================================================================

/// Permanently delete a library folder. Same security envelope as
/// `export_library_zip` — the path must canonicalize to something inside the
/// user's Documents directory, so a malicious or misconfigured caller can't
/// rm-rf arbitrary host paths.
///
/// Surfaces the concrete reason for any failure (path doesn't resolve, lives
/// outside Documents, busy file handle, permission denied) so the UI can show
/// the user what to fix instead of a generic "delete failed".
#[tauri::command]
pub async fn delete_library(path: String) -> Result<(), String> {
    let raw = PathBuf::from(&path);
    let p = raw.canonicalize().map_err(|e| {
        let msg = format!(
            "could not resolve library path '{}': {}. The folder may have already been moved or deleted.",
            raw.display(),
            e
        );
        tracing::warn!("delete_library: {}", msg);
        msg
    })?;
    if !p.is_dir() {
        let msg = format!("'{}' is not a folder — refusing to delete.", p.display());
        tracing::warn!("delete_library: {}", msg);
        return Err(msg);
    }
    let docs = dirs::document_dir().ok_or_else(|| {
        let msg = "cannot resolve your Documents directory — delete refused for safety.".to_string();
        tracing::warn!("delete_library: {}", msg);
        msg
    })?;
    if !p.starts_with(&docs) {
        let msg = format!(
            "library must live inside Documents ({}); '{}' is outside.",
            docs.display(),
            p.display()
        );
        tracing::warn!("delete_library: {}", msg);
        return Err(msg);
    }
    let stage = force_remove_dir(&p).await.map_err(|e| {
        tracing::warn!("delete_library: {} (path={})", e, p.display());
        e
    })?;
    tracing::info!("delete_library: removed {} via {}", p.display(), stage);
    Ok(())
}
