pub mod commands;
pub mod engine;
pub mod events;
pub mod mcp;
pub mod sources;
pub mod util;

/// The Tauri `identifier` from `tauri.conf.json`. Used to locate the app's
/// config dir before (or without) an `AppHandle` — the safe-render marker and
/// the library-root file the MCP server reads. Keep in sync with
/// `tauri.conf.json` (checked by `tests/linux_identifiers_in_sync.rs`).
pub const APP_IDENTIFIER: &str = "com.webworldwide.documentfinder";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Manager;
    // MCP server mode: this same binary, launched by an AI app (Claude
    // Desktop / Claude Code / any MCP client) with `--mcp`, serves the user's
    // libraries over stdio. Intercept before any window/runtime starts.
    if std::env::args().nth(1).as_deref() == Some(mcp::MCP_ARG) {
        mcp::serve_stdio();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // WebKitGTK reads WEBKIT_DISABLE_DMABUF_RENDERER once, at library init —
    // long before any in-process Settings change could reach it. Must run
    // before the Tauri builder (hence before any window/webview) exists.
    #[cfg(target_os = "linux")]
    maybe_relaunch_in_safe_render_mode();

    install_panic_hook();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::default_library_dir,
            commands::set_library_root,
            commands::start_run,
            commands::retry_run,
            commands::cancel_run,
            commands::list_libraries,
            commands::open_library,
            commands::list_library_docs,
            commands::search_library,
            commands::read_doc_markdown,
            commands::export_library_zip,
            commands::export_library_markdown,
            commands::copy_for_ai,
            commands::mcp_info,
            commands::install_mcp_claude_desktop,
            commands::reveal_in_finder,
            commands::open_path,
            commands::run_log_info,
            commands::run_log_tail,
            commands::delete_library,
            commands::purge_all_data,
            commands::restart_app,
            commands::local_searxng_port,
            commands::get_safe_render_mode,
            commands::set_safe_render_mode,
        ])
        .setup(|app| {
            // Bundled PDFium lives in the resource dir (see tauri.conf.json
            // bundle.resources); tell the extractor where to look.
            if let Ok(res) = app.path().resource_dir() {
                engine::extract::set_pdfium_dirs(vec![res.join("pdfium"), res]);
            }
            // Start the in-process SearXNG-compatible HTTP server on a
            // random localhost port. Once running, `SearxngPoolSource` and
            // any external code can hit `http://127.0.0.1:<port>/search`
            // without needing Docker or a Python SearXNG install.
            let handle = app.handle().clone();
            let client = std::sync::Arc::new(sources::make_client());
            // Back the local server with a *no-pool* aggregator: the pool
            // prefers the local server, so a pool-backed one here would recurse.
            let meta_search = std::sync::Arc::new(
                sources::meta_search::MetaSearchSource::new_without_pool_fallback(
                    client,
                    Some(handle),
                ),
            );
            tauri::async_runtime::spawn(async move {
                match sources::local_searxng::spawn_server(meta_search).await {
                    Ok(port) => tracing::info!("local SearXNG listening on 127.0.0.1:{port}"),
                    Err(e) => tracing::error!("failed to start local SearXNG: {e}"),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running document-finder");
}

/// Marker file path for the opt-in "safe rendering mode" (see
/// `maybe_relaunch_in_safe_render_mode`). Lives under the same
/// `app_config_dir()` Tauri resolves at runtime (`dirs::config_dir()/<id>`),
/// so toggling it from Settings (`commands::set_safe_render_mode`) and
/// "Erase app data" (which now purges `app_config_dir()`, see
/// `commands::purge_all_data`) both target the same file.
#[cfg(target_os = "linux")]
fn safe_render_marker_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_IDENTIFIER).join("safe-render-mode"))
}

/// WebKitGTK's blank-window-on-launch issue (some older GPU drivers + system
/// WebKitGTK combos) has no reliable auto-detection — only a documented
/// manual workaround (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, see README). Since
/// the env var must be set before WebKitGTK initializes, and this crate
/// forbids `unsafe_code` (so `std::env::set_var`, `unsafe fn` since Rust
/// 1.82, can't be called here), we instead re-exec this same binary with the
/// var added — `exec()` is a *safe* fn that replaces the process image, so
/// the new process starts with the var already set. Works identically for
/// .deb/.rpm/AppImage/Flatpak (no sandbox boundary crossed — same PID/mount
/// namespace).
///
/// Two opt-ins: the env var `DF_SAFE_RENDER=1` (a one-shot escape hatch, same
/// idea as setting `WEBKIT_DISABLE_DMABUF_RENDERER` directly) or a marker
/// file written by the Settings "Safe rendering mode" toggle (persists across
/// launches). Checking whether the target var is already set makes this
/// idempotent — the re-exec'd process inherits it, so this cannot loop.
#[cfg(target_os = "linux")]
fn maybe_relaunch_in_safe_render_mode() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }
    let env_opt_in = std::env::var("DF_SAFE_RENDER").as_deref() == Ok("1");
    let marker_opt_in = safe_render_marker_path().is_some_and(|p| p.is_file());
    if !env_opt_in && !marker_opt_in {
        return;
    }
    tracing::info!(
        "safe rendering mode requested; relaunching with WEBKIT_DISABLE_DMABUF_RENDERER=1"
    );
    let Ok(exe) = std::env::current_exe() else {
        tracing::warn!("safe rendering mode: could not resolve current_exe; continuing without it");
        return;
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(exe)
        .args(&args)
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .exec(); // only returns on failure
    tracing::warn!("safe rendering mode: re-exec failed ({err}); continuing without it");
}

fn install_panic_hook() {
    // This hook fires for both std::thread panics AND tokio::spawn task panics
    // (tokio routes task panics through the standard panic machinery before
    // propagating the JoinError). Logging here gives us a stack-trace line
    // even when the JoinHandle is dropped rather than awaited.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        tracing::error!(target: "panic", "panic at {}: {}", location, payload);
        prev(info);
    }));
}
