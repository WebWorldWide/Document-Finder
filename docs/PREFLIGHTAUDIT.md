# Document Finder — Pre-flight Audit
**Date:** 2026-05-18  
**Branch:** v2-stabilize-ship  
**Status:** Working map — check items off as Parts 1–6 proceed

---

## Dead Rust Symbols
`cargo check` passes cleanly — zero dead_code or unused warnings at audit time.  
*Resolved:* N/A

---

## Orphaned SearXNG References (full list with file:line)

### Rust
- [x] `src-tauri/src/sources/searxng.rs` — entire file (`SearXNGSource` struct + impl)
- [x] `src-tauri/src/sources/mod.rs:12` — `pub mod searxng;`
- [x] `src-tauri/src/sources/mod.rs:46` — `"searxng"` in SOURCE_IDS
- [x] `src-tauri/src/sources/mod.rs:129-139` — `"searxng"` arm in `build_source()`
- [x] `src-tauri/src/commands.rs` — `setup_searxng` command (Docker-based setup)
- [x] `src-tauri/src/lib.rs:33` — `commands::setup_searxng` registered in invoke_handler
- [x] `src-tauri/src/events.rs` — `EV_SEARXNG_LOG`, `EV_SEARXNG_STAGE`, `SearxngLogPayload`, `SearxngStagePayload`

### Frontend
- [x] `src/components/SearxngSetupPanel.tsx` — entire file (Docker setup UI)
- [x] `src/stores/settings.ts:66-69` — `searxngUrl` field + Docker setup comment
- [x] `src/stores/run.ts:310-311` — searxng source_options pass-through
- [x] `src/components/SettingsView.tsx` — references to SearxngSetupPanel

### Documentation
- [ ] `README.md` — check for Docker/SearXNG setup instructions (Part 6.9)

---

## Hard-coded Color Values (full list)

### Components (must move to tokens — Part 3 target)
- [ ] `src/components/WelcomeDialog.tsx:49` — `oklch(1 0 0 / 0.95)` inline shadow
- [ ] `src/components/WelcomeDialog.tsx:61` — `text-white` on surface-glossy
- [ ] `src/components/WelcomeDialog.tsx:65` — inline shadow with oklch
- [ ] `src/components/LibraryView.tsx:141` — `oklch(0.32 0.05 50)` background
- [ ] `src/components/LibraryView.tsx:142` — `oklch(0.85 0.05 50)` icon color
- [ ] `src/components/LibraryView.tsx:145` — `oklch(0.85 0.05 50)` text color
- [ ] `src/components/Sidebar.tsx:34` — `text-white` on surface-glossy
- [ ] `src/components/Sidebar.tsx:38` — inline shadow with oklch
- [ ] `src/components/SearxngSetupPanel.tsx:163` — inline color-mix (will be deleted)

### Styles (definitions — correct location; tokens are defined here)
- These are DEFINITIONS in globals.css — correct and expected, not violations

---

## Missing aria-labels (components that need them)

These are partially fixed since the original audit. Remaining gaps:
- [ ] `FindTab.tsx` — source toggle buttons (icon-only in some views)
- [ ] `FindTab.tsx` — issues accordion `aria-expanded` attribute
- [ ] `FindTab.tsx` — Cancel button icon needs `aria-hidden`
- [ ] `LibraryView.tsx:101-118` — Export and Show buttons inside library cards
- [ ] `SettingsView.tsx:95` — Library Folder input missing `<label>`/`aria-labelledby`
- [ ] `App.tsx:12` — drag region div missing `aria-hidden="true"`
- [x] `Sidebar.tsx:55,70` — open library and reveal buttons have aria-label
- [x] `FindTab.tsx:269,291` — Dismiss buttons have aria-label
- [x] `WelcomeDialog.tsx:53` — Dismiss button has aria-label
- [x] `LibraryView.tsx:112` — Dismiss button has aria-label

---

## Dead / Orphaned Tauri Commands

### Registered in Rust but need verification in frontend:
- `setup_searxng` — called from `SearxngSetupPanel.tsx` (will be deleted in Part 2)
- `delete_library` — registered, need to verify frontend caller

### Commands registered (lib.rs invoke_handler):
1. `default_library_dir`
2. `start_run`
3. `cancel_run`
4. `list_libraries`
5. `open_library`
6. `export_library_zip`
7. `reveal_in_finder`
8. `run_log_info`
9. `run_log_tail`
10. `setup_searxng` ← DELETE in Part 2
11. `list_models`
12. `is_embedding_loaded`
13. `download_model`
14. `cancel_model_download`
15. `delete_model`
16. `delete_library`

---

## Unused npm Packages

`package.json` is lean — no obvious unused packages.  
`depcheck` not run (no npx in scope); all deps are referenced by known config files.

---

## Unused Cargo Crates

`cargo check` passes clean. Potential unused at audit time:  
- `fs2 = "0.4"` — verify it's referenced in at least one .rs file  
- `dashmap = "6"` — verify active use  
*(Part 6 will do full udeps sweep)*

---

## Database Schema Baseline

**Source:** `src-tauri/src/engine/db.rs`

No migration system exists. Schema is created inline at startup via `CREATE TABLE IF NOT EXISTS`.

### Table: `runs`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PRIMARY KEY AUTOINCREMENT | |
| query | TEXT NOT NULL | |
| folder_path | TEXT NOT NULL | |
| created_at | DATETIME DEFAULT CURRENT_TIMESTAMP | |

### Table: `documents`
| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PRIMARY KEY AUTOINCREMENT | |
| run_id | INTEGER NOT NULL REFERENCES runs(id) | |
| url | TEXT NOT NULL | |
| title | TEXT NOT NULL | |
| source | TEXT NOT NULL | |
| authors | TEXT DEFAULT '[]' | JSON array |
| year | TEXT | |
| abstract_ | TEXT | |
| identifier | TEXT | |
| file_path | TEXT | |
| file_size_bytes | INTEGER DEFAULT 0 | |
| extracted_text | TEXT | |
| score | REAL DEFAULT 0.0 | |
| created_at | DATETIME DEFAULT CURRENT_TIMESTAMP | |

### Indexes
- `idx_documents_run_id ON documents(run_id)`
- `idx_runs_created_at ON runs(created_at DESC)`

### Migration status
No versioned migration system. **Part 5 must create one** before adding columns for download limits.

---

## Leaked unlisten Callbacks (specific locations)

- [x] `src/main.tsx:7-9` — `listenAll` result is stored in `window._dfUnlisten` ✓
- [ ] `src/components/SearxngSetupPanel.tsx:53,64,72` — `unsubs: UnlistenFn[]` array populated but `onCleanup` not confirmed; needs verification that unsubs.forEach(u=>u()) is called on cleanup. Will be deleted in Part 2.
- [ ] `src/App.tsx:14` — `onMount` with no listen calls (just `api.defaultLibraryDir`) — not a listener leak

---

## Key Bugs Found (for Part 1)

### PRIMARY: Poisonable std::sync::Mutex in embeddings.rs
- **File:** `src-tauri/src/ai/embeddings.rs:127`
- `static MODEL: OnceLock<Arc<Mutex<EmbeddingModel>>>` uses `std::sync::Mutex`
- Line 172: `.map_err(|_| anyhow::anyhow!("embedding mutex poisoned"))?` — confirmed crash path
- Any ONNX inference panic while holding the lock = permanent mutex poison = all subsequent searches fail

### SECONDARY: OnceLock prevents reset
- `embeddings.rs` and `llm.rs` both use `OnceLock` which cannot be reset after initialization
- After mutex poison, there's no way to recover without app restart
- Fix requires replacing `OnceLock<Arc<Mutex<...>>>` with `RwLock<Option<Arc<...>>>` to allow reset

### TERTIARY: AsyncMutex in llm.rs can deadlock if spawn_blocking panics
- `llm.rs:167`: `static MODEL: OnceLock<Arc<AsyncMutex<LlmModel>>>`
- Tokio AsyncMutex doesn't poison but if blocking thread panics while holding it, lock never releases

---

## Part Checklist

| Part | Status |
|------|--------|
| 0 - Pre-flight audit | ✅ COMPLETE |
| 1 - Crash fix | 🔲 TODO |
| 2 - Meta-search hardening | 🔲 TODO |
| 3 - Theme overhaul | 🔲 TODO |
| 4 - CI/CD | 🔲 TODO |
| 5 - Security audit fixes | 🔲 TODO |
| 6 - Cleanup sweep | 🔲 TODO |
