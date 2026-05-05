//! Tauri commands invoked from the Solid.js frontend.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::engine::db::init_db;
use crate::engine::runlog;
use crate::engine::{run_pipeline, RunRequest};
use crate::events::{ErrorPayload, EV_ERROR};

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

#[tauri::command]
pub async fn setup_searxng(_app: AppHandle) -> Result<String, String> {
    // Verify Docker is installed and running.
    let docker_info = tokio::process::Command::new("docker")
        .arg("info")
        .output()
        .await
        .map_err(|_| "Docker is not installed. Install Docker Desktop from https://www.docker.com/products/docker-desktop/".to_string())?;

    if !docker_info.status.success() {
        return Err("Docker is not running. Please start Docker Desktop and try again.".to_string());
    }

    // Short-circuit if container is already running.
    if let Ok(out) = tokio::process::Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", "document-finder-searxng"])
        .output()
        .await
    {
        if String::from_utf8_lossy(&out.stdout).trim() == "true" {
            return Ok("SearXNG is already running.\nSEARXNG_URL=http://localhost:8080".to_string());
        }
    }

    // Pull the image.
    let pull = tokio::process::Command::new("docker")
        .args(["pull", "searxng/searxng:latest"])
        .output()
        .await
        .map_err(|e| format!("docker pull failed: {}", e))?;

    if !pull.status.success() {
        let stderr = String::from_utf8_lossy(&pull.stderr).to_string();
        return Err(format!("docker pull failed: {}", stderr.trim()));
    }

    // Remove any stale container.
    let _ = tokio::process::Command::new("docker")
        .args(["rm", "-f", "document-finder-searxng"])
        .output()
        .await;

    // Start the container.
    let run = tokio::process::Command::new("docker")
        .args([
            "run", "-d",
            "--name", "document-finder-searxng",
            "--restart", "unless-stopped",
            "-p", "8080:8080",
            "searxng/searxng:latest",
        ])
        .output()
        .await
        .map_err(|e| format!("docker run failed: {}", e))?;

    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr).to_string();
        return Err(format!("docker run failed: {}", stderr.trim()));
    }

    // Wait up to 30 s for SearXNG to respond using reqwest (avoids curl PATH dependency).
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();
    for _ in 0..15 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let ok = http.get("http://localhost:8080/")
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().as_u16() < 500)
            .unwrap_or(false);
        if ok {
            return Ok("SearXNG started successfully.\nSEARXNG_URL=http://localhost:8080".to_string());
        }
    }

    Ok("SearXNG container started — it may need a few more seconds to be fully ready.\nSEARXNG_URL=http://localhost:8080".to_string())
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
