# Document Finder — developer & agent guide

The single source of developer truth for this repo. User-facing docs live in
[`README.md`](README.md); this file is everything else (architecture, build
internals, verification, release, icon regen, conventions). Written to double as
context for coding agents.

Document Finder is a Tauri 2 desktop app: a **Rust** backend (`src-tauri/`) and a
**Solid.js + Vite + TypeScript** frontend (`src/`). It searches open-access
sources in parallel, downloads documents, extracts text, and stores everything in
per-query SQLite libraries on disk.

## Architecture

### Frontend (`src/`)
- `App.tsx` / `main.tsx` — shell and entry; `main.tsx` registers the global Tauri
  event listeners (kept in `window._dfUnlisten` for cleanup).
- `components/` — `FindTab`, `LiveResultsView`, `LibraryView`, `SettingsView`,
  `Sidebar`, `PipelineStrip`, `MetaSearchHealthBar`, `ModelStatusBadge`,
  `ModelDownloadCard`, `ThemePicker`, `WelcomeDialog`.
- `stores/` — Solid signals: `ui` (routing), `theme`, `settings`, `models`,
  `pipeline`, `run` (active search state).
- `lib/` — `tauri.ts` (typed `invoke` bindings + `RunRequest`/`Document` types),
  `events.ts` (event names + payloads, mirror of `src-tauri/src/events.rs`),
  `utils.ts` (`SOURCE_LABELS`, `sourceColor`, formatters).
- `styles/globals.css` — Tailwind v4 (`@import "tailwindcss"`) plus the design
  tokens. Themes are driven by `data-theme` on `<html>`: `warm-light`,
  `warm-dark`, `apple-light`, `apple-dark`. Per-source colors are
  `--color-source-<id>` tokens.

### Backend (`src-tauri/src/`)
- `lib.rs` — Tauri builder, `invoke_handler!` (the command allowlist), and the
  setup hook that spawns the in-process SearXNG server. Also installs a panic
  hook that logs `tokio::spawn` task panics.
- `commands.rs` — every `#[tauri::command]`. Must stay in sync with
  `permissions/app.toml` (enforced by the `permissions_in_sync` test).
- `events.rs` — event-name constants and payload structs emitted to the UI.
- `sources/` — `trait Source` (`mod.rs`) with one module per backend:
  `arxiv`, `openalex`, `semantic_scholar`, `internet_archive`, `doaj`,
  `gutenberg`, and the web layer: `meta_search` (aggregator) over six HTML
  scrapers (`duckduckgo`, `bing_html`, `brave_html`, `mojeek_html`,
  `marginalia_html`, `startpage_html`) with `searxng_pool` and `local_searxng`
  as fallbacks. `web_common` holds shared scraping helpers. `mod.rs` also has
  `build_source`, `SOURCE_IDS`, the `Document` struct, `make_client`, and
  `get_with_retry` (exponential backoff on 429/5xx).
- `engine/` — `orchestrator` (the search→download→extract→rank pipeline),
  `query` (sub-query expansion + slugging), `db` (SQLite, `CREATE TABLE IF NOT
  EXISTS`, no migrations yet), `downloader` (concurrent download with a
  `Content-Length` size cap and `tokio::select!` cancellation), `extract`
  (PDF/EPUB text, panic-guarded), `ranking` (TF-IDF + optional semantic
  rerank), `dedup`, `authority`, `citation_graph`, `manifest` (legacy
  `manifest.json` read path only), `runlog` (JSONL run log; `read_tail` reads a
  bounded window).
- `ai/` — optional, feature-gated. `embeddings` (bge-small via `fastembed`/ort,
  `ai-embeddings`), `llm` (Qwen 2.5 3B via `llama-cpp-2`, `ai-llm`), plus
  `registry`, `downloader` (model fetch + SHA256), `storage`, `state`. Models
  live behind resettable `RwLock<Option<…>>` singletons so a poisoned/failed
  model can be reset without restarting the app.
- `util/` — `path_safety` (confine library paths to Documents) and `url_safety`
  (`validate_url`: https-only, no credentials, public-IP-only SSRF check).

### Web meta-search & local SearXNG
`MetaSearchSource` fans out to the six scrapers concurrently with a per-engine
circuit breaker (3 failures → skip 5 min) and emits `df:meta_search_health`. When
every circuit is open it falls back to `SearxngPoolSource`, which **prefers the
in-process local server** (`http://127.0.0.1:<port>`, SSRF-exempt) before a
public `searx.space` instance.

The in-process server (`local_searxng.rs`) is started at app launch and is backed
by `MetaSearchSource::new_without_pool_fallback` — **not** the pool-backed one —
because the pool prefers the local server, so a pool-backed aggregator there would
recurse local → pool → local forever. Every public instance URL is SSRF-validated
before use. No Docker, no Python, no setup.

## Build & run

```bash
./run.ps1     # Windows
./run.sh      # macOS / Linux  (installs pnpm if missing)
```

Manual:

```bash
pnpm install --frozen-lockfile --ignore-scripts   # see "esbuild" note below
pnpm tauri dev
pnpm tauri build                                   # native installers
```

- **Feature flags** (`src-tauri/Cargo.toml`): `default = custom-protocol +
  ai-embeddings + ai-llm`. The AI features build llama.cpp + ONNX Runtime from
  source (needs `cmake` + `clang`/LLVM; first build is 10–25 min). For fast dev
  rebuilds without AI:
  `cargo build --no-default-features --features=custom-protocol`.
- **esbuild / pnpm 11**: pnpm 11 hard-errors on unapproved build scripts.
  esbuild's binary ships in its platform package, so its postinstall isn't
  needed — install with `--ignore-scripts` (CI does this on every platform).

## Verify (mirrors CI)

Frontend:
```bash
pnpm lint          # eslint, max-warnings 15
pnpm format:check  # prettier
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest
pnpm build         # vite
```

Backend (fast lane — skips the heavy AI build):
```bash
cargo fmt   --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --features=custom-protocol --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml --no-default-features --features=custom-protocol
```

`ci.yml` runs all of the above on every PR to `main`; the full 3-OS Tauri build
matrix runs on push-to-`main` or a `full-build`-labelled PR. `release.yml` builds
installers on a `v*` tag (Linux on **ubuntu-24.04** — the ONNX Runtime binary
needs glibc 2.39+ C23 symbols).

## Conventions

- **Adding a source**: create `src-tauri/src/sources/<name>.rs` implementing
  `Source` (`name()` + `search() -> BoxStream<Result<Document>>`), register it in
  `mod.rs` (`pub mod`, `SOURCE_IDS`, `build_source`), add a `SOURCE_LABELS` entry
  and a `--color-source-<id>` token on the frontend, and enable it by default in
  `DEFAULT_ENABLED_SOURCES` if appropriate.
- **Adding/removing a command**: edit `commands.rs` and the `invoke_handler!` in
  `lib.rs`, then mirror it in `permissions/app.toml` — the `permissions_in_sync`
  test fails otherwise.
- **Events**: keep `src-tauri/src/events.rs` and `src/lib/events.ts` in sync.
- **External URLs**: anything user- or network-supplied must pass
  `util::url_safety::validate_url` before a request.
- `unsafe` is forbidden (`[lints.rust] unsafe_code = "forbid"`); clippy runs at
  `-D warnings` in CI.

## Release

1. Bump the version in **all three**: `package.json`, `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json`.
   ```bash
   git commit -am "chore: bump version to x.y.z"
   git tag vx.y.z
   git push origin main --tags
   ```
2. `release.yml` builds unsigned installers — macOS (Apple Silicon `.dmg`),
   Linux (`.deb` + `.AppImage`, glibc 2.39+), Windows (`.exe` + `.msi`) — and
   attaches them to a **draft** GitHub Release. Review, edit notes, publish.

The builds are unsigned, so first launch needs a manual allow:
- **macOS** — System Settings → Privacy & Security → **Open Anyway**, or
  `xattr -dr com.apple.quarantine "/Applications/Document Finder.app"`. Verify
  with `codesign -dvvv "/Applications/Document Finder.app"` (expect "not signed").
- **Windows** — SmartScreen → **More info** → **Run anyway**.

## Icon regeneration

Master artwork lives in `icons/` (not shipped at runtime):
`Document Finder Icon.svg` is the vector master; `Document Finder MacOS.png`
(1024×1024) is the raster master. After editing a master, regenerate every
runtime size into `src-tauri/icons/`:

```bash
pnpm tauri icon "icons/Document Finder MacOS.png"
```

## Product website

`site/` is a static product page deployed to GitHub Pages by
`.github/workflows/pages.yml` on push to `main`. One-time setup: repo
**Settings → Pages → Source: GitHub Actions**. It reuses the app's palette with a
blue-green accent sampled from the logo and the logo SVG itself (`site/logo.svg`).
