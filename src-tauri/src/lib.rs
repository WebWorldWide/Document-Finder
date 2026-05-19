pub mod ai;
pub mod commands;
pub mod engine;
pub mod events;
pub mod sources;
pub mod util;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    install_panic_hook();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .manage(ai::AiState::default())
        .invoke_handler(tauri::generate_handler![
            commands::default_library_dir,
            commands::start_run,
            commands::cancel_run,
            commands::list_libraries,
            commands::open_library,
            commands::export_library_zip,
            commands::reveal_in_finder,
            commands::run_log_info,
            commands::run_log_tail,
            commands::list_models,
            commands::is_embedding_loaded,
            commands::download_model,
            commands::cancel_model_download,
            commands::delete_model,
            commands::delete_library,
            commands::reset_ai_state,
            commands::local_searxng_port,
        ])
        .setup(|app| {
            // Start the in-process SearXNG-compatible HTTP server on a
            // random localhost port. Once running, `SearxngPoolSource` and
            // any external code can hit `http://127.0.0.1:<port>/search`
            // without needing Docker or a Python SearXNG install.
            let handle = app.handle().clone();
            let client = std::sync::Arc::new(sources::make_client());
            let meta_search = std::sync::Arc::new(sources::meta_search::MetaSearchSource::new(
                client,
                Some(handle),
            ));
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
