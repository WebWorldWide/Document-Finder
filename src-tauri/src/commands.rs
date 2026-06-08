//! Tauri commands invoked from the Solid.js frontend.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::ai;
use crate::ai::registry;
use crate::ai::state::{snapshot, AiState, ModelInfo, ModelStatus};
use crate::engine::db::init_db;
use crate::engine::runlog;
use crate::engine::{run_pipeline, RunRequest};
use crate::events::{ErrorPayload, EV_ERROR};
use crate::sources::USER_AGENT;
use crate::util::path_safety::{library_root as default_library_root, safe_within_root};

/// HTTP client tuned for huge model downloads — connect_timeout fails fast on
/// dead URLs but there is NO overall .timeout, because the default
/// `make_client()` uses 60s which would silently kill a 2GB download
/// mid-stream.
fn make_model_download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(30))
        .redirect(crate::sources::safe_redirect_policy())
        .dns_resolver(std::sync::Arc::new(crate::util::url_safety::PublicOnlyResolver))
        // Intentionally NO .timeout — model files take many minutes.
        .build()
        .expect("model http client")
}

#[derive(Default)]
pub struct AppState {
    /// Active run's cancellation token, if any.
    pub current_cancel: Mutex<Option<CancellationToken>>,
    /// The user-configured library root, validated + canonicalized once when
    /// set from the frontend (`set_library_root`). All path commands confine
    /// to this so a library relocated to e.g. `D:\Research` can still be
    /// opened/exported/deleted. `None` falls back to the default
    /// `~/Documents/Document Finder` root.
    pub library_root: Mutex<Option<PathBuf>>,
}

/// Resolve the confinement root every path command must stay inside: the
/// user-configured root if one has been set + validated, else the default
/// `~/Documents/Document Finder`. Keeping this server-side (rather than trusting
/// a root passed per-call from the renderer) preserves the anti-traversal
/// guarantee while still honoring a custom location.
fn confinement_root(state: &AppState) -> Result<PathBuf, String> {
    let guard = state.library_root.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(root) => Ok(root.clone()),
        None => default_library_root(),
    }
}

/// Reject directories too broad to serve as the confinement root. That root is
/// the deletion boundary for `delete_library`, `export_library_zip`, and
/// `purge_all_data(include_library)`, so letting a (renderer-supplied) root be a
/// drive/filesystem root, the home dir, or the Documents dir itself would turn a
/// later purge into a recursive wipe of unrelated user data. A dedicated
/// *subfolder* is fine. `path` is expected to be already canonicalized.
fn is_sensitive_root(path: &Path) -> bool {
    // Filesystem root or a bare drive root (e.g. `C:\`) has no parent.
    if path.parent().is_none() {
        return true;
    }
    for special in [dirs::home_dir(), dirs::document_dir()]
        .into_iter()
        .flatten()
    {
        if special.canonicalize().ok().as_deref() == Some(path) {
            return true;
        }
    }
    false
}

/// Persist + validate the user's chosen library root. Called by the frontend on
/// startup and whenever the Settings library path changes, so the security
/// envelope for open/export/delete matches where downloads actually go.
#[tauri::command]
pub fn set_library_root(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let raw = PathBuf::from(&path);
    if !raw.is_absolute() {
        return Err("Library folder must be an absolute path.".into());
    }
    // Don't let a buggy/compromised renderer trigger arbitrary deep directory
    // creation: only create the folder if it doesn't already exist AND its
    // parent does — i.e. create at most one new level under an existing dir.
    if !raw.exists() {
        match raw.parent() {
            Some(parent) if parent.is_dir() => {
                std::fs::create_dir(&raw).map_err(|e| {
                    format!("Could not create library folder '{}': {e}", raw.display())
                })?;
            }
            _ => {
                return Err(format!(
                    "Library folder '{}' doesn't exist and its parent is missing — create the folder first.",
                    raw.display()
                ));
            }
        }
    }
    let canonical = raw
        .canonicalize()
        .map_err(|e| format!("Could not resolve library folder '{}': {e}", raw.display()))?;
    if is_sensitive_root(&canonical) {
        return Err(format!(
            "'{}' is too broad to use as the library folder — choose a dedicated subfolder.",
            canonical.display()
        ));
    }
    let mut root = state.library_root.lock().unwrap_or_else(|e| e.into_inner());
    *root = Some(canonical.clone());
    Ok(canonical.to_string_lossy().to_string())
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
    // Confine the download output to the configured library root. `out_dir`
    // arrives from the renderer and the pipeline `create_dir_all`s under it, so
    // without this a compromised/buggy renderer could redirect every downloaded
    // file outside the library (e.g. into C:\Windows or /etc). The per-query
    // subfolder is already slugged (`safe_folder`), making `out_dir` the only
    // traversal vector. The normal flow always pre-creates this folder
    // (`default_library_dir` / `set_library_root`), so the canonicalizing check
    // succeeds for legitimate runs and rejects anything outside the root.
    let root = confinement_root(&state)?;
    safe_within_root(Path::new(&req.out_dir), &root).map_err(|e| {
        format!("Refusing to run: the library folder is outside the allowed root. {e}")
    })?;

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
                let _ = app2.emit(
                    EV_ERROR,
                    ErrorPayload {
                        message: e.to_string(),
                    },
                );
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
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
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
pub async fn list_libraries(
    state: State<'_, AppState>,
    root: String,
) -> Result<Vec<LibraryInfo>, String> {
    // Confine the scanned root to the configured library root. Without this a
    // compromised/buggy renderer could enumerate + stat-walk arbitrary
    // directories and induce a `library.db` write into any folder holding a
    // legacy manifest.json (the migration path below). Mirrors the confinement
    // every other library command applies.
    let confined = confinement_root(&state)?;
    let root = PathBuf::from(root);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let root = safe_within_root(&root, &confined)?;
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
pub async fn open_library(state: State<'_, AppState>, path: String) -> Result<LibraryInfo, String> {
    let root = confinement_root(&state)?;
    let p = safe_within_root(&PathBuf::from(&path), &root)?;
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
pub fn export_library_zip(
    state: State<'_, AppState>,
    args: ExportArgs,
) -> Result<ExportResult, String> {
    let root = confinement_root(&state)?;
    let src = safe_within_root(&PathBuf::from(&args.folder), &root)?;
    if !src.is_dir() {
        return Err(format!("not a folder: {}", args.folder));
    }
    // The destination is legitimately OUTSIDE the library root (the user picks
    // it via the native save dialog — Desktop, Downloads, etc.), so it can't be
    // confined like the source. But it still arrives from the renderer, so don't
    // let it fabricate arbitrary directory trees or clobber a non-file: require
    // an absolute path whose parent folder already exists, and a plain-file (or
    // absent) target. This removes the prior `create_dir_all` write primitive.
    let dest_path = PathBuf::from(&args.dest);
    if !dest_path.is_absolute() {
        return Err("Export destination must be an absolute path.".into());
    }
    match dest_path.parent() {
        Some(parent) if parent.is_dir() => {}
        _ => {
            return Err(
                "Export destination's folder doesn't exist — pick an existing folder.".into(),
            )
        }
    }
    if dest_path.exists() && !dest_path.is_file() {
        return Err("Export destination already exists and isn't a file.".into());
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

#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let raw = PathBuf::from(&path);
    // Block URI schemes (e.g. "file:///etc") before canonicalize is called.
    if path.contains("://") {
        return Err("path must not be a URI".to_string());
    }
    // Reveal is a benign "show in the OS file manager" action, and its targets
    // are legitimately OUTSIDE the library root — most importantly an exported
    // ZIP the user just saved via the native save dialog (e.g. ~/Documents/foo.zip,
    // Desktop, Downloads, another drive). Confining it to the library root made
    // every Library/Discover export fail with "… is outside the allowed root"
    // *after* the ZIP was already written. We still canonicalize to reject
    // nonexistent/garbage paths and normalize before handing it to the OS.
    let p = raw
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{}': {e}", raw.display()))?;
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
        // explorer.exe `/select,` does not understand the `\\?\` verbatim prefix
        // that `canonicalize()` returns — handed one it fails to highlight the
        // target and opens the default folder instead. Strip the prefix back to
        // a normal Win32 path.
        let s = p.to_string_lossy();
        let simplified = s
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .or_else(|| s.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or_else(|| s.to_string());
        // Pass /select and the path as two separate args so commas in the
        // path cannot split the argument and confuse Explorer's parser.
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&simplified)
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

/// Whether the embedding model (managed entirely by fastembed) is loaded
/// in-process. The first semantic-rerank call triggers a download into
/// fastembed's own cache; this command lets the UI surface that lifecycle
/// without us shipping a duplicate registry entry that goes stale.
#[tauri::command]
pub fn is_embedding_loaded() -> bool {
    #[cfg(feature = "ai-embeddings")]
    {
        crate::ai::embeddings::is_loaded()
    }
    #[cfg(not(feature = "ai-embeddings"))]
    {
        false
    }
}

/// Whether the embedding model is already cached on disk (so it loads without a
/// network fetch). Best-effort; lets Settings show a clear status separate from
/// `is_embedding_loaded` (which reports in-memory readiness this session).
#[tauri::command]
pub fn embedding_downloaded(_app: AppHandle) -> bool {
    #[cfg(feature = "ai-embeddings")]
    {
        crate::ai::embeddings::is_downloaded(&_app)
    }
    #[cfg(not(feature = "ai-embeddings"))]
    {
        false
    }
}

/// Start a one-time background download + load of the embedding model so the
/// user can warm it from Settings instead of waiting for the first semantic
/// search. No-op if already loaded/warming; the UI learns it's ready via the
/// `df:model_status` event.
#[tauri::command]
pub fn warm_embedding(_app: AppHandle) {
    #[cfg(feature = "ai-embeddings")]
    {
        crate::ai::embeddings::warm_in_background(_app);
    }
}

#[tauri::command]
pub fn list_models(app: AppHandle, state: State<'_, AiState>) -> Result<Vec<ModelInfo>, String> {
    // Wrap in catch_unwind so a panic anywhere in `from_entry` (mutex poison,
    // path resolution, registry access) becomes a real error string the UI
    // can show instead of a hung promise.
    //
    // AppHandle and State refs aren't UnwindSafe (they hold Arcs into mutex
    // chains); AssertUnwindSafe is appropriate because if a panic does occur
    // we're going to surface and bail, not continue using the borrowed state.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| snapshot(&app, &state)));
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
    let entry =
        registry::find(&model_id).ok_or_else(|| format!("unknown model id: {}", model_id))?;

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
    let entry =
        registry::find(&model_id).ok_or_else(|| format!("unknown model id: {}", model_id))?;

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
#[cfg(target_os = "windows")]
fn clear_readonly_recursive(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            clear_readonly_recursive(&entry.path())?;
        }
    }
    let mut perms = metadata.permissions();
    if perms.readonly() {
        perms.set_readonly(false);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

async fn force_remove_dir(path: &Path) -> Result<&'static str, String> {
    // Stage 1: `remove_dir_all` with a short retry/backoff loop.
    //
    // On Windows a library we just listed or opened can still have transient
    // handles open on its SQLite files (`-wal`/`-shm` from WAL mode), or be
    // held for a beat by the search indexer / antivirus, so the first
    // `remove_dir_all` fails with a sharing violation (PermissionDenied,
    // os error 32). Those clear within a few hundred milliseconds, so retry a
    // few times before falling back. Harmless on every platform.
    let mut stage1_msg = String::new();
    for attempt in 0..6u32 {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => return Ok("remove_dir_all"),
            // Already gone — a previous attempt may have half-succeeded.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok("already gone");
            }
            Err(e) => {
                stage1_msg = format!("remove_dir_all: {} ({:?})", e, e.kind());
                tracing::info!(
                    "force_remove_dir stage 1 attempt {} failed for {}: {}",
                    attempt + 1,
                    path.display(),
                    stage1_msg
                );
                // Growing backoff (120ms, 240ms, …); no sleep after the last try.
                if attempt < 5 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        120 * u64::from(attempt + 1),
                    ))
                    .await;
                }
            }
        }
    }

    {
        // Stage 2 on Windows: clear read-only attributes, then retry.
        #[cfg(target_os = "windows")]
        {
            let path_clone = path.to_path_buf();
            let clear_res = tokio::task::spawn_blocking(move || {
                clear_readonly_recursive(&path_clone)
            })
            .await
            .map_err(|e| format!("JoinError: {}", e))
            .and_then(|res| res.map_err(|e| format!("clear_readonly: {}", e)));

            match clear_res {
                Ok(()) => {
                    tracing::info!("Cleared read-only attributes for {}", path.display());
                    if let Err(e2) = tokio::fs::remove_dir_all(path).await {
                        let stage2_msg = format!("after-clear remove_dir_all: {}", e2);
                        tracing::info!(
                            "force_remove_dir stage 2 failed for {}: {}",
                            path.display(),
                            stage2_msg
                        );
                        return Err(format!(
                            "all delete strategies failed. stage 1 (remove_dir_all): {}, stage 2 (after clear_readonly): {}",
                            stage1_msg, stage2_msg
                        ));
                    }
                    return Ok("clear_readonly + remove_dir_all");
                }
                Err(e) => {
                    return Err(format!(
                        "failed to clear read-only attributes: {}. stage 1 (remove_dir_all): {}",
                        e, stage1_msg
                    ));
                }
            }
        }

        // Stage 2 on macOS: strip macOS ACLs + xattrs, then retry. (`chmod +a` ACLs or a
        // `com.apple.quarantine` xattr can deny `delete` even with no open
        // handle.)
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
            Ok("chmod -RN + xattr -rc")
        }

        // Non-macOS, non-Windows: retries exhausted; surface the last error.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(stage1_msg)
        }
    }
}

// =============================================================================
// AI state reset
// =============================================================================

/// Drop loaded AI singletons so the next search re-initializes them from
/// scratch. Called automatically by the frontend when an EV_ERROR event
/// fires, so users can retry without restarting the app after an inference
/// crash.
/// Returns the bound port of the in-process SearXNG-compatible server.
/// `None` if the server failed to bind on startup.
#[tauri::command]
pub fn local_searxng_port() -> Option<u16> {
    crate::sources::local_searxng::local_port()
}

#[tauri::command]
pub async fn reset_ai_state() -> Result<(), String> {
    #[cfg(feature = "ai-embeddings")]
    crate::ai::embeddings::reset_embedding_model();
    #[cfg(feature = "ai-llm")]
    crate::ai::llm::reset_llm_model();
    tracing::info!("AI state reset by user");
    Ok(())
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
pub async fn delete_library(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let root = confinement_root(&state)?;
    // `safe_within_root` canonicalizes BOTH the path and the root, so this keeps
    // the Windows `\\?\` verbatim-prefix fix that the old hand-rolled check
    // needed — while moving the boundary to the configured library root (matching
    // open/export) instead of the broader whole-Documents tree.
    let p = safe_within_root(&PathBuf::from(&path), &root).map_err(|e| {
        tracing::warn!("delete_library: {}", e);
        e
    })?;
    if !p.is_dir() {
        let msg = format!("'{}' is not a folder — refusing to delete.", p.display());
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

// =============================================================================
// Clean uninstall — purge all app data
// =============================================================================

/// What [`purge_all_data`] removed (or failed to remove). Best-effort: a failure
/// on one location is recorded here and the remaining locations are still tried.
#[derive(Debug, Serialize)]
pub struct PurgeReport {
    pub removed: Vec<String>,
    pub failed: Vec<String>,
}

/// Erase everything Document Finder writes to disk, for a clean uninstall on any
/// OS (the part no native installer removes).
///
/// SAFETY: this takes **no path from the caller** — every directory it deletes is
/// derived server-side from the app identifier (`app_data_dir`, which holds the
/// downloaded AI model weights + the fastembed cache + config), the run-log
/// location ([`runlog::log_path`]), and — only when `include_library` is set —
/// the confined library root ([`confinement_root`], which already knows a custom
/// root configured via `set_library_root`). So a compromised/buggy renderer can't
/// turn this into an arbitrary `rm -rf`. The document library is user content, so
/// it is preserved unless `include_library` is explicitly true.
#[tauri::command]
pub async fn purge_all_data(
    app: AppHandle,
    state: State<'_, AppState>,
    include_library: bool,
) -> Result<PurgeReport, String> {
    let mut targets: Vec<PathBuf> = Vec::new();

    // 1. App data dir (identifier-keyed): downloaded LLM weights, the fastembed
    //    BGE/ONNX cache, and any config nested under the same folder.
    match app.path().app_data_dir() {
        Ok(dir) => targets.push(dir),
        Err(e) => tracing::warn!("purge_all_data: no app_data_dir: {e}"),
    }
    // 1b. Local app data dir (identifier-keyed). On Windows this is the
    //     %LOCALAPPDATA% WebView2/EBWebView store (localStorage, IndexedDB, HTTP
    //     cache) — a *different* folder from the Roaming app_data_dir above, so
    //     without this the in-app purge silently leaves it behind (matching
    //     scripts/uninstall.ps1). On macOS/Linux it resolves to the same path as
    //     app_data_dir, so the loop's is_dir() guard just skips the duplicate.
    match app.path().app_local_data_dir() {
        Ok(dir) => targets.push(dir),
        Err(e) => tracing::warn!("purge_all_data: no app_local_data_dir: {e}"),
    }
    // 2. Run-log directory (a different base dir on every OS).
    if let Some(log_dir) = runlog::log_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
        targets.push(log_dir);
    }
    // 3. The document library (downloaded files + per-query SQLite) — user
    //    content, so only when the user explicitly opts in.
    if include_library {
        match confinement_root(&state) {
            Ok(root) => targets.push(root),
            Err(e) => tracing::warn!("purge_all_data: could not resolve library root: {e}"),
        }
    }

    let mut report = PurgeReport {
        removed: Vec::new(),
        failed: Vec::new(),
    };
    for dir in targets {
        let path_str = dir.display().to_string();
        if !dir.is_dir() {
            continue; // Nothing there (or already gone) — treat as success.
        }
        match force_remove_dir(&dir).await {
            Ok(stage) => {
                tracing::info!("purge_all_data: removed {} via {}", path_str, stage);
                report.removed.push(path_str);
            }
            Err(e) => {
                tracing::warn!("purge_all_data: failed to remove {} ({})", path_str, e);
                report.failed.push(format!("{path_str}: {e}"));
            }
        }
    }
    Ok(report)
}
