# Document Finder — Security & Bug Audit

**Reviewed:** 2026-05-05  
**Depth:** Deep (full cross-file analysis)  
**Reviewer:** Claude (adversarial review)  
**Stack:** Tauri 2 · Solid.js · Rust · SQLite (rusqlite) · reqwest

---

## Summary

The codebase is well-structured and demonstrates awareness of several threat classes (parameterized SQL, magic-byte validation, cancellation tokens, content-type rejection). However, the audit surfaced multiple issues across path safety, network trust, command injection surface, type safety across the IPC boundary, and subtle logic bugs. The most urgent items are the unvalidated path arguments passed to OS shell commands (`reveal_in_finder` on Windows, `export_library_zip`), the unbounded download size, the unvalidated SearXNG URL, and the WAL-mode non-atomicity window. None of these require architectural rethink; all have straightforward fixes.

---

## CRITICAL

### C-01 — Path Traversal in `reveal_in_finder` (Windows)

**File:** `src-tauri/src/commands.rs:468–472`

**Issue:** On Windows the path is interpolated into a `/select,{path}` string and passed as a single `arg()` to `explorer.exe`. If the path contains a comma, the argument splits and the content after the comma is interpreted as a second argument by Explorer's command-line parser. More critically, neither the Windows nor the macOS code validates that `path` is inside any expected directory. A path like `/etc/passwd` or `C:\Windows\System32\cmd.exe` passes the `p.exists()` check and is opened in the shell. Because `reveal_in_finder` is an unrestricted Tauri command reachable from JavaScript, any renderer-side XSS or supply-chain compromise of the frontend can call it with arbitrary paths.

```rust
// Current (Windows path, commands.rs:470)
std::process::Command::new("explorer")
    .arg(format!("/select,{}", p.display()))  // comma in path splits the arg

// Fix: pass as two separate args so the comma is not special
std::process::Command::new("explorer")
    .arg("/select,")          // note trailing comma is part of the flag
    // Actually correct fix:
    .arg(format!("/select,\"{}\"", p.display()))
// Or better: validate path is under the expected library root before accepting it.
```

Additionally, add a guard so that `reveal_in_finder` only accepts paths under the app's library root (from `default_library_dir`). The same should apply to `open_library` and `list_libraries`.

---

### C-02 — No Download Size Limit — Disk Exhaustion

**File:** `src-tauri/src/engine/downloader.rs:214–240`

**Issue:** The downloader streams bytes to disk with no upper bound on total file size. A malicious server (or a legitimate server returning an unexpectedly large response) can fill the user's disk. The `content-length` header is read but only used for progress display, never as a rejection threshold. A `max_total = 500` run with 8 concurrent downloads, each feeding an unlimited stream, can write hundreds of gigabytes.

```rust
// Fix: reject or abort when downloaded bytes exceed a sane limit, e.g. 500 MB per file
const MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;

// Inside the chunk loop:
downloaded += chunk.len() as u64;
if downloaded > MAX_FILE_BYTES {
    drop(file);
    let _ = tokio::fs::remove_file(&out).await;
    return DownloadOutcome::Failed(
        format!("file too large (> {} bytes)", MAX_FILE_BYTES)
    );
}
```

Optionally also pre-reject if `content-length` header > threshold before writing a single byte.

---

### C-03 — Unvalidated SearXNG URL Enables SSRF

**File:** `src-tauri/src/sources/mod.rs:109–111`, `src/stores/settings.ts:23`

**Issue:** The `instance_url` for SearXNG is taken directly from user settings, stored in `localStorage`, and passed verbatim to `reqwest` with no URL validation. A user can set it to `http://169.254.169.254/latest/meta-data/` (AWS IMDS), `file:///etc/passwd`, or any internal network resource. While Tauri desktop apps don't have the same SSRF risk profile as a server, this still allows the app to be used to probe internal infrastructure the user's machine can reach, and `file://` URIs via reqwest are blocked by default but scheme validation is still best practice.

```rust
// In sources/mod.rs build_source(), validate before constructing SearXNGSource:
"searxng" => {
    let raw_url = _options.instance_url
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    // Reject non-http(s) schemes and obviously internal targets
    let parsed = url::Url::parse(&raw_url)
        .ok()
        .filter(|u| u.scheme() == "http" || u.scheme() == "https")?;
    Some(Box::new(searxng::SearXNGSource::new(client, parsed.to_string())))
}
```

---

### C-04 — `export_library_zip` Writes Outside Expected Directories Without Validation

**File:** `src-tauri/src/commands.rs:263–301`

**Issue:** Both `args.folder` (source) and `args.dest` (destination) are arbitrary paths provided by the caller with no confinement check. The `folder` argument is validated only with `is_dir()`, meaning it can be any directory on the filesystem (e.g., `/etc`, `C:\Windows`). The `dest` argument has its parent `create_dir_all`'d, allowing creation of directories anywhere. The ZIP writer will then faithfully ZIP the entire specified directory tree, potentially including credential files or OS data if the path is not the library root. Because Tauri's Rust commands run with the full OS user privileges, there is no sandbox to catch this.

The frontend always passes `state.folder` (the run output folder), but the IPC boundary is not enforced — any JS with access to the IPC (e.g., after a supply-chain compromise) can call it with `/Users/victim` as the folder.

```rust
// Fix: canonicalize and assert folder is under the library root
pub fn export_library_zip(args: ExportArgs) -> Result<ExportResult, String> {
    let src = PathBuf::from(&args.folder)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let library_root = dirs::document_dir()
        .ok_or("cannot resolve Documents")?
        .join("Document Finder")
        .join("library");
    if !src.starts_with(&library_root) {
        return Err("folder is outside the library root".to_string());
    }
    // ... rest of function
}
```

---

## HIGH

### H-01 — SQLite WAL Mode Not Guaranteed on Concurrent Connections

**File:** `src-tauri/src/engine/db.rs:4–8`, `src-tauri/src/engine/orchestrator.rs:366–382`

**Issue:** `init_db` sets WAL mode via `PRAGMA journal_mode = WAL`. WAL mode is persistent in the database file once set, but there is a race: the very first call to `init_db` (in the run setup at `orchestrator.rs:80`) sets WAL. However, up to `concurrency` (default 8, max unbounded) download tasks each call `DbManager::new` → `init_db` concurrently on the same database file. Between the first connection setting WAL and the file persisting it, other connections opening in the default rollback-journal mode can cause `SQLITE_BUSY` or silent data corruption. In practice SQLite handles this gracefully for same-process WAL connections, but the more dangerous issue is that each `spawn_blocking` task uses `INSERT OR REPLACE` which triggers a write transaction; with multiple concurrent connections and no shared connection pool, write-write conflicts will produce `SQLITE_BUSY` errors that are silently dropped (`let _ = mgr.insert_document(...)`).

```rust
// Fix: Use a single Arc<Mutex<Connection>> or a connection pool (r2d2-sqlite),
// passed through to all tasks. Alternatively use a dedicated writer task receiving
// documents over a tokio::sync::mpsc channel.

// Minimum-viable fix: add busy_timeout pragma
conn.pragma_update(None, "busy_timeout", 5000)?; // 5 second retry window
```

---

### H-02 — `inFlight` Map Key Is URL — Duplicate URLs Cause Lost State

**File:** `src/stores/run.ts:143–155`

**Issue:** The `inFlight` map is keyed on `ev.payload.url`. If the same document URL appears twice in the candidates list (which can happen because `seen_urls` deduplication happens in Phase 1 but a document can legitimately have the same download URL from two different sources), both `download_started` events insert into `inFlight` for the same key. When the first `download_done` fires and removes the key, the second task's progress events are silently dropped and its `download_done` updates a key that no longer exists in `inFlight`. The `done`/`failed` counters are still incremented by the Rust backend correctly, but the UI's `active` counter undercounts and completed items may be missed in the display.

The `seen_urls` set in `orchestrator.rs:87` uses the document `url` field for deduplication. If two sources return the same PDF URL with different titles, the second one is filtered. However, if a URL seen in Phase 1 has been resolved to the same canonical URL later via redirect (the `reqwest` client follows redirects), there is no cross-source deduplication, and the symptom above can occur.

```typescript
// Fix: key inFlight on a stable per-task identifier, not the URL.
// Emit a unique task_id from Rust for each download task and use that as the key.
// Minimum fix: use `${url}-${Date.now()}` as the inFlight key.
```

---

### H-03 — `reveal_in_finder` Exposes Shell Injection Vector (macOS/Linux)

**File:** `src-tauri/src/commands.rs:459–484`

**Issue:** On macOS, `open -R <path>` is invoked with the path as a direct `.arg()` (no shell interpolation), so shell injection via the path string itself is not possible. However on Linux, `xdg-open` is called with `parent` (which is `p.parent()` or `/`). If `path` is something like `/tmp/; rm -rf ~` and `p.parent()` resolves to `/tmp`, the `.arg()` call passes it safely. The actual risk here is lower than C-01 because `.arg()` avoids shell expansion. That said, `xdg-open` on Linux can open URIs including `http://` and `file://` schemes — if `path` is attacker-controlled and set to a URI like `file:///etc/cron.d/evil`, `xdg-open` may process it as a URI not a path. Validate that the path is an absolute filesystem path before passing to xdg-open.

```rust
// Fix: validate path is absolute and does not contain URI scheme markers
if path.starts_with("/") && !path.contains("://") {
    // proceed
} else {
    return Err("invalid path".to_string());
}
```

---

### H-04 — ArXiv URL Construction Allows arxiv ID Injection

**File:** `src-tauri/src/sources/arxiv.rs:257–263`

**Issue:** The arXiv search query is constructed by joining keywords with `+AND+` and embedding the result directly into the URL string without percent-encoding:

```rust
let q = keywords.iter()
    .map(|k| format!("all:{}", k))
    .collect::<Vec<_>>()
    .join("+AND+");
let url = format!("{}?search_query={}&start={}&max_results={}", BASE, q, start, per_page);
```

Keywords come from `parse_query` on user input. A user who types a query containing `&max_results=0&start=0&search_query=` can override the intended query parameters. The `WORD_RE` in `query.rs` (`r"(?:[A-Za-z][A-Za-z'-]+|\b\d{4}\b)"`) filters to alphanumeric-only tokens that cannot contain `&` or `=`, so in practice this specific injection path is closed. However, the pattern of building URLs via string formatting rather than using a proper query-string encoder creates a latent risk if the regex ever changes. The correct fix is to use `reqwest`'s `.query()` builder:

```rust
let resp = client.get(BASE)
    .query(&[
        ("search_query", &q),
        ("start", &start.to_string()),
        ("max_results", &per_page.to_string()),
    ])
    .send()
    .await;
```

---

### H-05 — `write_zip_recursive` Does Not Guard Against Symlink Traversal

**File:** `src-tauri/src/commands.rs:303–347`

**Issue:** `write_zip_recursive` recurses into all subdirectories found via `read_dir` + `path.is_dir()`. On Unix, `is_dir()` follows symlinks, so a symlink in the library folder pointing outside (e.g., `library/my-run/evil -> /etc`) causes the ZIP to include files from outside the intended directory. Since `folder_size_bytes` also follows the same pattern (line 104), an attacker who can place a symlink in the library directory (or who tricks the app into creating one via a downloaded file — unlikely but worth noting) can exfiltrate arbitrary files.

```rust
// Fix: use symlink_metadata instead of is_dir(), and skip symlinks
let meta = std::fs::symlink_metadata(&path)?;
if meta.file_type().is_symlink() {
    continue;  // never follow symlinks in zip export
}
if meta.is_dir() { ... }
```

---

## MEDIUM

### M-01 — `DownloadDonePayload` `identifier` Field Missing from TypeScript Interface

**File:** `src/lib/events.ts:58–71`, `src-tauri/src/events.rs:83–94`

**Issue:** `DownloadDonePayload` and `DownloadFailedPayload` in Rust use `#[serde(flatten)]` on the `Document` struct. `Document` has an `identifier` field (`Option<String>`). The TypeScript `DownloadDonePayload` and `DownloadFailedPayload` interfaces do not include `identifier`. This is a type-safety gap; if code is added that reads `payload.identifier` in TypeScript, it will be `undefined` but the TypeScript compiler won't warn. The field is also absent from `events.ts` interfaces but present in the Rust `Document` struct, which is used for the DB `identifier` column.

```typescript
// Fix: add to both DownloadDonePayload and DownloadFailedPayload in events.ts
identifier?: string;
```

---

### M-02 — `run_log_tail` Has No Size Guard on Full File Read

**File:** `src-tauri/src/engine/runlog.rs:110–125`

**Issue:** `read_tail` reads the entire JSONL file into memory with `read_to_string`, then reverses all lines and takes `max`. For a long-running user who has accumulated thousands of runs, this file could be megabytes or more. The frontend calls `run_log_tail(200)` which then allocates the full file just to discard most of it. There is also no maximum size on the log file itself — it grows without bound for the lifetime of the app.

```rust
// Fix: read only the tail of the file using seek, or add a rotation/truncation policy.
// Minimum: cap the log file at e.g. 10MB by rotating on open.
// Interim: at least add a file size check before read_to_string:
if let Ok(meta) = std::fs::metadata(&path) {
    if meta.len() > 50 * 1024 * 1024 {
        return Vec::new(); // refuse to load enormous log
    }
}
```

---

### M-03 — Settings Stored in Unencrypted `localStorage` Including Arbitrary URLs

**File:** `src/stores/settings.ts:5–9`

**Issue:** `localStorage` persists settings including `searxngUrl`, `libraryRoot`, and numeric caps in plaintext in the browser's WebView storage. While this is a desktop app and the threat model is different from a web app, on macOS the WebView data directory (under `~/Library/WebKit/`) is readable by any process running as the user. A local malware process can modify `localStorage` to set `searxngUrl` to an attacker-controlled endpoint without the user's knowledge, causing the next search run to leak queries to that endpoint. The risk is compounded by the lack of URL validation (C-03).

The library root path in settings is also used without canonicalization: `list_libraries(root)` is called with the raw `settings.libraryRoot` string, which could include `../` sequences if a user crafted the setting manually, potentially listing arbitrary directories.

```typescript
// Fix: canonicalize libraryRoot on load/save; validate searxngUrl is http(s) on save
export function saveSettings() {
  const toSave = { ...settings };
  // basic URL validation before persisting
  try { new URL(toSave.searxngUrl); } catch { toSave.searxngUrl = "http://localhost:8080"; }
  localStorage.setItem(LS_KEY, JSON.stringify(toSave));
}
```

---

### M-04 — Cancellation Does Not Interrupt In-Progress HTTP Chunks

**File:** `src-tauri/src/engine/downloader.rs:216–230`

**Issue:** Cancellation is checked at the top of the semaphore-acquire block and between every chunk write. However, `stream.next().await` has no timeout and no cancellation integration — a hung server that sends the HTTP headers then stalls mid-body will block that task indefinitely. The Tokio `select!` macro is needed to race the stream poll against the cancellation token, otherwise a cancelled run can still have tasks blocked for the full `reqwest` client timeout (60 seconds, `make_client` line 121). With 8 concurrent downloads all hung, cancellation appears to hang the UI for up to 60 seconds.

```rust
// Fix: use tokio::select! in the chunk loop
loop {
    let chunk_res = tokio::select! {
        c = stream.next() => c,
        _ = cancel.cancelled() => {
            drop(file);
            let _ = tokio::fs::remove_file(&out).await;
            return DownloadOutcome::Cancelled;
        }
    };
    let Some(chunk_res) = chunk_res else { break };
    // ... rest of loop
}
```

---

### M-05 — `safe_folder` Slug Collision Allows Directory Confusion

**File:** `src-tauri/src/engine/query.rs:116–128`

**Issue:** `safe_folder` is used as the output directory name derived purely from the query string. Two different queries that produce the same 60-character slug (after lowercasing and hyphen-collapsing) will write into the same folder. For example, "machine learning for NLP tasks" and "machine learning for NLP tasks!!!" produce identical slugs. This means a second run with a colliding query appends documents to the existing run's folder and inserts into the same `library.db` with a new `run_id`, mixing documents from different queries in one library. More seriously, if a first run is in progress and a second run begins with a colliding slug, both runs write to the same folder concurrently.

```rust
// Fix: append a timestamp or short random suffix to guarantee uniqueness
use std::time::{SystemTime, UNIX_EPOCH};
let ts = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
format!("{}-{}", slug, ts)
```

---

### M-06 — `list_libraries` Accepts Arbitrary `root` Path (No Confinement)

**File:** `src-tauri/src/commands.rs:112–193`

**Issue:** `list_libraries` takes a `root: String` from the frontend without validation and calls `read_dir` on it, then opens SQLite databases and reads their contents. An attacker with control of the frontend can enumerate any directory on the filesystem to locate `.db` files, or cause the function to iterate a directory with thousands of entries causing a long-running blocking operation on the main thread (the function uses `spawn_blocking` properly, but the directory scan itself is not). The same applies to `open_library(path: String)`.

```rust
// Fix: validate root is under the user's expected library root directory
fn assert_under_library_root(path: &Path) -> Result<(), String> {
    let library_root = dirs::document_dir()
        .ok_or("cannot resolve Documents")?
        .join("Document Finder");
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical.starts_with(&library_root) {
        Ok(())
    } else {
        Err("path is outside the library root".to_string())
    }
}
```

---

### M-07 — `handleOpenLibrary` Silently Swallows All Errors

**File:** `src/components/FindTab.tsx:51–57`

**Issue:** The `catch {}` block in `handleOpenLibrary` catches any error from `api.openLibrary` and discards it with no user feedback. If the library database is corrupted, missing, or inaccessible, the user clicks "Open Library" and nothing happens — no error message, no state change. This leaves the UI in a silently broken state.

```typescript
// Fix: surface the error to the user
async function handleOpenLibrary() {
  if (!rs().folder) return;
  try {
    const info = await api.openLibrary(rs().folder!);
    uiStore.setActiveLibrary(info);
    uiStore.setView("library");
  } catch (e) {
    // Show error to user, e.g. set an error signal
    setExportError(String(e)); // re-use existing error signal or add a dedicated one
  }
}
```

---

### M-08 — `runlog` Lock Acquisition Silently Drops Log Events

**File:** `src-tauri/src/engine/runlog.rs:99`

**Issue:** `LOG_LOCK.lock().ok()` discards the `LockResult` silently. If the mutex is poisoned (due to a panic in another thread that held the lock), all subsequent log events are silently dropped because the `_guard` binding is `None` and the append proceeds unguarded. The pattern should use `unwrap_or_else(|e| e.into_inner())` (as used elsewhere in the codebase) to recover from poisoning, or at minimum log a warning.

```rust
// Current
let _guard = LOG_LOCK.lock().ok();

// Fix
let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
```

---

### M-09 — `inFlight` Entries Not Cleaned Up on `cancelled` Event

**File:** `src/stores/run.ts:208–224`

**Issue:** When the `cancelled` or `complete` event fires, `setState` updates `running`, `folder`, etc., but does **not** clear `inFlight`. Any downloads that were in-flight at the time of cancellation (those that reached `DownloadOutcome::Cancelled` in Rust and never emit `download_done` or `download_failed`) leave stale entries in `inFlight`. The UI will continue to show these ghost downloads with their last-known progress bytes forever, until the next `reset()` call. The `active` counter also stays inflated.

```typescript
// Fix: in the 'complete' / 'cancelled' case, clear inFlight:
case "complete":
case "cancelled":
  setState({
    running: false,
    inFlight: {},      // add this
    active: 0,         // add this
    folder: ev.payload.folder,
    ...
  });
```

---

## LOW

### L-01 — `connect-src` CSP Allows All HTTPS

**File:** `src-tauri/tauri.conf.json:27`

**Issue:** `"connect-src": "'self' https:"` permits the WebView to make `fetch()` or XHR requests to any HTTPS URL. In a Tauri app, network requests should go through the Rust backend (via `invoke`), not directly from the WebView. Allowing `connect-src https:` means compromised frontend code can exfiltrate data directly to external servers without going through Rust. The `img-src 'self' data: https:` is similarly broad.

```json
// More restrictive CSP:
"connect-src": "'self'",
"img-src": "'self' data:"
```

If specific CDN hosts are genuinely needed (e.g., for fonts or avatar images), enumerate them explicitly rather than using a wildcard.

---

### L-02 — `curl` Binary Path Not Verified in `setup_searxng`

**File:** `src-tauri/src/commands.rs:439–447`

**Issue:** `setup_searxng` invokes `curl` by bare name (relies on `PATH`). On Windows, `curl` may not be in `PATH` or a malicious binary named `curl` could be earlier in the path. Additionally, `curl` is called to check SearXNG health — using `reqwest` (already a dependency) would be safer and cross-platform:

```rust
// Fix: replace the curl health check with a reqwest request
let ok = reqwest::get("http://localhost:8080/")
    .await
    .map(|r| r.status().is_success())
    .unwrap_or(false);
```

---

### L-03 — `folder_size_bytes` Has No Symlink Guard

**File:** `src-tauri/src/commands.rs:95–109`

**Issue:** `folder_size_bytes` recurses into subdirectories identified by `meta.is_dir()`, which follows symlinks. A symlink cycle (A → B → A) would cause infinite recursion and a stack overflow. This is unlikely in a user's Documents folder but is a crash vector if a malicious downloaded file somehow creates such a symlink.

```rust
// Fix: use symlink_metadata to detect and skip symlinks
let Ok(meta) = entry.metadata() else { continue };
if meta.file_type().is_symlink() { continue; }
```

---

### L-04 — `read_tail` Reversal Is O(n) in Total Log Lines, Not in `max`

**File:** `src-tauri/src/engine/runlog.rs:115–124`

**Issue:** The implementation reads all lines, calls `.rev()`, `.take(max)`, then `.collect()` and `.reverse()`. The `.rev()` on a `Lines` iterator still requires consuming the entire content string (it's not a true random-access reverse). For a 100 000-line log, this allocates a Vec of 100 000 `Value`s before taking 200. The correct approach is to scan from the end of the file using `seek`.

This is flagged as LOW because it is a correctness/reliability issue (large logs cause transient high memory use) rather than a security issue.

---

### L-05 — `settings.ts` Unsafe Type Casts on `localStorage` Load

**File:** `src/stores/settings.ts:17–24`

**Issue:** All settings are cast with `as` from the raw `unknown` value returned by `JSON.parse`. If the stored JSON has `perSource: "banana"` (e.g., from a previous version with different types, or manual tampering), `(saved.perSource as number)` returns `"banana"` typed as `number`. The `numInput` handler in `SettingsView` validates on write but not on read, so malformed stored settings propagate silently into the run request.

```typescript
// Fix: validate/coerce loaded values before use
const perSource = typeof saved.perSource === "number" && saved.perSource > 0
  ? saved.perSource
  : 100;
```

---

### L-06 — `DuckDuckGo` `looks_like_doc` Is Too Permissive

**File:** `src-tauri/src/sources/duckduckgo.rs:65–84`

**Issue:** The fallback at line 80–83 returns `true` for any URL that doesn't explicitly end in `.html`/`.htm` and doesn't contain `/abs/` or `/article/`. This matches general web pages, social media links, and other non-document URLs that pass through the relevance filter if their titles happen to contain a keyword. The downloader's content-type check will ultimately reject HTML landing pages, but many unnecessary network requests are made first.

---

### L-07 — Unbounded `inFlight` Record Growth Under High Concurrency

**File:** `src/stores/run.ts:143–155`

**Issue:** `inFlight` is a plain object (`Record<string, InFlight>`). The `completed` array is capped at 500 and `log` at 200, but `inFlight` has no cap. With `concurrency = 8` and `max_total = 500`, at peak there are 8 entries. However if the backend emits `download_started` for a URL and then emits neither `download_done` nor `download_failed` (which can happen for `DownloadOutcome::Cancelled` — see M-09), the entry is never removed. Over multiple runs without a page reload, stale entries accumulate.

---

### L-08 — `ExportArgs` `dest` Can Overwrite Existing Files Without Warning

**File:** `src-tauri/src/commands.rs:272`

**Issue:** `File::create(&dest_path)` silently truncates and overwrites any existing file at `dest`. The Tauri file dialog `save()` used in the frontend prompts the user to confirm before overwriting on some platforms, but this confirmation happens in JavaScript and is not enforced at the Rust layer. A direct `invoke("export_library_zip", ...)` call skips the dialog entirely, allowing overwrite of arbitrary files (subject to the other path-validation gaps noted in C-04).

---

## INFO

### I-01 — `CSP` `style-src 'unsafe-inline'` Present

**File:** `src-tauri/tauri.conf.json:27`

**Issue:** `style-src 'self' 'unsafe-inline'` is required for Solid.js/Tailwind's runtime style injection. This is expected for the current build setup, but a future hardening step would be to switch to a nonce-based CSP or extract all styles at build time.

---

### I-02 — `index.html` References Deleted Entry Point

**File:** `index.html:12`

**Issue:** `index.html` contains `<script type="module" src="/src/main.tsx">`. The git status shows `src/main.tsx` is deleted (`D src/main.tsx`) and replaced by `src/main.ts`. If the build system resolves this correctly via Vite, it is not a runtime issue, but the mismatch between the HTML reference and the actual file is a maintenance hazard.

---

### I-03 — `package.json` Has Residual React/TSX References After Svelte Migration

**File:** `package.json` (modified), `index.html`, `src/App.tsx` (deleted)

**Issue:** The git status shows a migration from React/TSX (deleted `App.tsx`, `FindTab.tsx`, etc.) to a different framework (new `.svelte` files), but the actively compiled code is still `.tsx`. The `index.html` points at `main.tsx`. New `.svelte` files exist as untracked files alongside the active `.tsx` files. This dual-state means the project has dead/transitional code in flight. The `.svelte` stores (`run.svelte.ts`, `settings.svelte.ts`) are untracked and may have diverged from the `.ts` stores that are actually compiled.

---

### I-04 — No Upper Bound on `concurrency` Setting

**File:** `src/stores/settings.ts:20`, `src-tauri/src/engine/orchestrator.rs:237`

**Issue:** The settings UI sets `max="32"` on the concurrency input, but this is not enforced in the Rust backend. `Semaphore::new(req.concurrency.max(1))` accepts any positive value. A user who manually edits `localStorage` to set `concurrency: 999` will spawn 999 concurrent download tasks, saturating both the semaphore-less arXiv API and the local file system.

```rust
// Fix: clamp in the backend
let concurrency = req.concurrency.clamp(1, 32);
let semaphore = Arc::new(Semaphore::new(concurrency));
```

---

### I-05 — `ENTITY` Regex in `strip_html` Does Not Handle All Named HTML Entities

**File:** `src-tauri/src/engine/extract.rs:90`

**Issue:** The entity regex handles only 6 named entities (`amp`, `lt`, `gt`, `quot`, `nbsp`, `#39`, `apos`). HTML documents from sources like Internet Archive use hundreds of named entities (`&mdash;`, `&ndash;`, `&hellip;`, `&copy;`, `&reg;`, numeric entities like `&#8211;`, etc.). Unhandled entities are left as literal `&entity;` text in the extracted plain text, degrading the quality of AI-visible content.

---

_Reviewed: 2026-05-05_  
_Reviewer: Claude (gsd-code-reviewer) — adversarial stance_  
_Depth: deep_
