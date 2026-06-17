# Document Finder — developer & agent guide

The single source of developer truth for this repo. User-facing docs live in
[`README.md`](README.md); this file is everything else — architecture, build
internals, verification, release, conventions, third-party licenses, icon regen.
Written to double as context for coding agents.

Document Finder is a Tauri 2 desktop app: a **Rust** backend (`src-tauri/`) and a
**Solid.js + Vite + TypeScript** frontend (`src/`). It searches open-access
sources in parallel, downloads documents, extracts text, and stores everything in
per-query SQLite libraries on disk.

## Architecture

### Frontend (`src/`)
- `App.tsx` / `main.tsx` — shell and entry; `main.tsx` imports `stores/theme`
  (applies `data-*` before first paint) and registers the global Tauri event
  listeners (kept in `window.__dfUnlisten` for cleanup).
- `components/` — `Sidebar`, `FindTab` (Discover: hero query + rich Sources panel
  + live run card), `LibraryView`, `SettingsView`, `WelcomeDialog`, plus the
  shared editorial pieces `DocRow`, `SourcePanel`, `Sparkline`, `Banner`,
  `ProgressBar`, `Logo`, `ThemePicker`, and the AI/health widgets
  `ModelDownloadCard`, `ModelStatusBadge`, `MetaSearchHealthBar`. The live
  download stream + pipeline rail live inside `FindTab` (no separate
  `LiveResultsView`/`PipelineStrip`).
- `stores/` — Solid signals: `ui` (routing + known libraries), `theme`
  (`theme`/`accent`/`density`/`streamLayout`), `settings`, `models`, `pipeline`,
  `run` (active search state, incl. per-source live stats + cumulative bytes).
- `lib/` — `tauri.ts` (typed `invoke` bindings + `RunRequest`/`Document` types),
  `events.ts` (event names + payloads, mirror of `src-tauri/src/events.rs`),
  `utils.ts` (`SOURCE_LABELS`, `sourceColor`, `sourceDesc`, `ftypeFromPath`,
  formatters), `errors.ts` (plain-language error humanizers for the UI).
- `styles/globals.css` — Tailwind v4 (`@import "tailwindcss"`) plus the editorial
  design system (`df-*` classes). Themes are driven by **`data-theme`** on
  `<html>` (`paper` | `slate` | `midnight`), accents by **`data-accent`** (9), and
  density by **`data-density`** (`compact` | `regular`); defaults are
  `slate` + `sky`. Per-source colors are `--color-source-<id>` tokens. Legacy
  `--color-*` names are aliased to the editorial tokens, and a small compatibility
  shim maps a few old skeuomorphic class names for the not-fully-rewritten
  AI/welcome widgets.

### Backend (`src-tauri/src/`)
- `lib.rs` — Tauri builder, `invoke_handler!` (the command allowlist), and the
  setup hook that spawns the in-process SearXNG server. Also installs a panic
  hook that logs `tokio::spawn` task panics.
- `commands.rs` — every `#[tauri::command]`. Must stay in sync with
  `permissions/app.toml` (enforced by the `permissions_in_sync` test).
- `events.rs` — event-name constants, payload structs, and `classify_source_error`.
- `sources/` — `trait Source` (`mod.rs`) with one module per backend:
  `arxiv`, `openalex`, `semantic_scholar`, `internet_archive`, `doaj`,
  `gutenberg`, and the web layer: `meta_search` (aggregator) over six HTML
  scrapers (`duckduckgo`, `bing_html`, `brave_html`, `mojeek_html`,
  `marginalia_html`, `startpage_html`) with `searxng_pool` and `local_searxng`
  as fallbacks. `web_common` holds shared scraping helpers. `mod.rs` also has
  `build_source`, `SOURCE_IDS`, the `Document` struct, `make_client`, the browser
  `USER_AGENT`, and `get_with_retry` (exponential backoff on 429/5xx).
- `engine/` — `orchestrator` (the search→download→extract→rank pipeline),
  `query` (sub-query expansion + slugging), `db` (SQLite, `CREATE TABLE IF NOT
  EXISTS`, no migrations yet), `downloader` (concurrent download with a
  `Content-Length` size cap and `tokio::select!` cancellation; sends document-
  biased `Accept`/`Accept-Language`/`Referer` headers to cut 4xx rejections, and
  emits plain-language failure strings), `extract` (PDF/EPUB text, panic-guarded),
  `ranking` (TF-IDF + optional semantic rerank), `dedup`, `authority`,
  `citation_graph`, `manifest` (legacy read path only), `runlog` (JSONL run log).
- `ai/` — optional, feature-gated. `embeddings` (BGE-Small via `fastembed`/ort,
  `ai-embeddings`), `llm` (Qwen 2.5 1.5B via `llama-cpp-2`, `ai-llm`), plus
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

The in-process server (`local_searxng.rs`) is started at app launch, backed by
`MetaSearchSource::new_without_pool_fallback` — **not** the pool-backed one —
because the pool prefers the local server, so a pool-backed aggregator there would
recurse local → pool → local forever. The server registers its port **only once
`/healthz` answers** (with a background re-probe for slow starts) so a not-yet-
ready server never stalls the first query before pool fallback. Every public
instance URL is SSRF-validated. No Docker, no Python, no setup.

## Build & run

```bash
./run.ps1     # Windows
./run.sh      # macOS / Linux  (installs pnpm if missing)
```

**Clean rebuild — one command, every OS** (Windows / macOS / Linux):

```bash
pnpm clean-build         # clean app crate + caches, release-fast build
pnpm clean-build:full    # also rebuild llama.cpp + ONNX from scratch (15-25 min)
pnpm clean-build:dev     # debug build + hot reload (tauri dev)
```

`scripts/rebuild.mjs` cleans the Rust build + Vite/dist caches, reinstalls JS
deps, and runs the full Tauri build. Named `clean-build`, not `rebuild`, so it
doesn't collide with pnpm's built-in `rebuild` command.

Manual:

```bash
pnpm install --frozen-lockfile --ignore-scripts
pnpm tauri dev
pnpm tauri build                                   # native installers
```

- **Feature flags** (`src-tauri/Cargo.toml`): `default = custom-protocol +
  ai-embeddings + ai-llm`. The AI features build llama.cpp + ONNX Runtime from
  source (needs `cmake` + `clang`/LLVM with `libclang` on `PATH`; first build is
  10–25 min). For fast dev rebuilds without AI:
  `cargo build --no-default-features --features=custom-protocol`.
- **pnpm 11 / esbuild**: pnpm 11 refuses to silently skip esbuild's (unneeded)
  build script and hard-errors before every `pnpm <script>`. Resolved by
  answering its prompt in `pnpm-workspace.yaml`: `allowBuilds: { esbuild: false }`.
  Installs still pass `--ignore-scripts` (esbuild's binary ships in its platform
  package).

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
  `mod.rs` (`pub mod`, `SOURCE_IDS`, `build_source`), add `SOURCE_LABELS` +
  `sourceDesc` entries and a `--color-source-<id>` token on the frontend, and
  enable it by default in `DEFAULT_ENABLED_SOURCES` if appropriate.
- **Adding/removing a command**: edit `commands.rs` and the `invoke_handler!` in
  `lib.rs`, then mirror it in `permissions/app.toml` — the `permissions_in_sync`
  test fails otherwise.
- **Events**: keep `src-tauri/src/events.rs` and `src/lib/events.ts` in sync.
- **External URLs**: anything user- or network-supplied must pass
  `util::url_safety::validate_url` before a request.
- **Conditional CSS classes**: use Solid's `classList={{ ... }}`, not
  template-literal ternaries — `prettier-plugin-tailwindcss` strips the leading
  space inside a ternary string (`` `df-doc${c?" x":""}` `` → `"x"` → glued class).
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
2. A `create-release` job opens **one** draft Release for the tag (so the three
   matrix legs don't race to create duplicate drafts), then `release.yml` builds
   installers and uploads them to that draft via tauri-action's `releaseId` —
   macOS (Apple Silicon `.dmg`, **ad-hoc signed**), Linux (`.deb`, `.rpm`,
   `.AppImage`, and a `.flatpak` from `packaging/flatpak/`, glibc 2.39+), Windows
   (`.exe` + `.msi`). Review, edit notes, publish.

The macOS build is **ad-hoc signed** (`bundle.macOS.signingIdentity = "-"` in
`tauri.conf.json`, also set via `APPLE_SIGNING_IDENTITY: '-'` in `release.yml`).
Ad-hoc signing is required on Apple Silicon — without it a downloaded (quarantined)
arm64 app gets the unrecoverable Gatekeeper *"is damaged"* verdict, **not** the
milder "unidentified developer" prompt. Ad-hoc signing removes the "damaged" wall
but is **not** notarization, so first launch still needs a manual allow:
- **macOS** — right-click the app → **Open**, then confirm. If macOS still
  refuses: `xattr -dr com.apple.quarantine "/Applications/Document Finder.app"`.
  (Zero-friction opens would require a paid Developer ID cert + notarization.)
- **Windows** — SmartScreen → **More info** → **Run anyway**.

The Flatpak builds against the **GNOME runtime** (`org.gnome.Platform`/`Sdk`),
not the bare freedesktop runtime, because only the GNOME runtime ships
`libwebkit2gtk-4.1` (a Tauri Linux app dlopen's it). The flatpak job runs an
`ldd` smoke check that fails the release if the chosen runtime is missing the lib
— bump `runtime-version` (manifest) and the `flatpak install` line together.

## Uninstall / data purge

Clean uninstall is three layers (no native uninstaller can remove the user's
document library, and shouldn't auto-nuke `~/Documents`):
- **`purge_all_data` command** (`commands.rs`) — the in-app "Settings → Danger
  zone → Erase app data". It takes **no path argument** by design: it only
  deletes dirs derived server-side from the app identifier (`app_data_dir()` —
  AI models + fastembed cache + config), `runlog::log_path()`'s parent (the per-
  OS log dir), and — only when `include_library` is set — `confinement_root()`
  (which knows a custom library root). It reuses `force_remove_dir` and is
  best-effort (returns a `PurgeReport { removed, failed }`). Registered in the
  usual four places (`commands.rs`, `lib.rs`, `permissions/app.toml`,
  `src/lib/tauri.ts`).
- **`scripts/uninstall.{ps1,sh}`** — standalone per-user data wipe for when the
  app is already gone. They only know the **default** `~/Documents/Document
  Finder`; keep their hard-coded identifier (`com.webworldwide.documentfinder`)
  and log paths in sync with `runlog.rs` if those ever change.
- **NSIS hook is intentionally deferred** — Tauri's generated Windows uninstaller
  already ships an opt-in "Delete application data" checkbox covering
  `%APPDATA%`/`%LOCALAPPDATA%`; a force hook would only remove that choice and
  still must not touch the Documents library.

## Icon regeneration

Master artwork lives in `icons/` (not shipped at runtime):
`Document Finder Icon.svg` is the vector master (also used as the in-app logo via
`src/components/Logo.tsx`); `Document Finder MacOS.png` (1024×1024) is the raster
master. After editing a master, regenerate every runtime size into
`src-tauri/icons/`:

```bash
pnpm tauri icon "icons/Document Finder MacOS.png"
```

## Product website

`site/` is a static product page deployed to GitHub Pages by
`.github/workflows/pages.yml` on push to `main`. One-time setup: repo
**Settings → Pages → Source: GitHub Actions**. It reuses the app's palette with a
blue-green accent sampled from the logo and the logo SVG itself (`site/logo.svg`).

## Third-party licenses

Document Finder is licensed under **GNU AGPL-3.0-or-later** (see [`LICENSE`](LICENSE)),
copyright Web World Wide. The third-party components below are informational and do
not modify any component's terms. License inventory last audited **2026-05-30**.

**AI model weights (downloaded at runtime, never bundled or redistributed).** The
optional local-LLM feature downloads weights on demand from Hugging Face to the
user's machine, under the publisher's license. The catalog lists only permissively
licensed models compatible with AGPL; each download is verified against a pinned
SHA-256 (`src-tauri/src/ai/registry.rs`):

| Model | Publisher | License | Source |
|---|---|---|---|
| Qwen 2.5 1.5B Instruct (GGUF, Q4_K_M) — *default* | Alibaba Cloud (Qwen) | Apache-2.0 | `Qwen/Qwen2.5-1.5B-Instruct-GGUF` |
| Qwen 2.5 0.5B Instruct (GGUF, Q4_K_M) | Alibaba Cloud (Qwen) | Apache-2.0 | `Qwen/Qwen2.5-0.5B-Instruct-GGUF` |

The text-embedding model (BGE-Small-EN-v1.5, MIT) is fetched and cached by the
`fastembed` library at runtime. Models with non-commercial/custom-restricted terms
(Qwen2.5-3B "qwen-research", Llama Community License, Mistral Research License) are
deliberately **excluded** so the defaults add no restrictions beyond ours.

**Rust dependencies** are all permissive / AGPL-compatible — predominantly
`MIT OR Apache-2.0`, with assorted MIT/BSD/Zlib/ISC/Unicode-3.0 and a few MPL-2.0
(file-level). No GPL-2.0-only/AGPL/SSPL/BUSL/CC-BY-NC/proprietary deps. Notable
native libs (all permissive): `llama-cpp-2`/`llama.cpp` (MIT), ONNX Runtime via
`ort` (MIT), `fastembed` (Apache-2.0), `rusqlite`/bundled SQLite (public domain /
MIT), `reqwest` + `rustls` (MIT / Apache-2.0).

**JavaScript / npm dependencies** are all permissive — mostly MIT, with ISC,
Apache-2.0, BSD, and a couple MPL-2.0 (`lightningcss`, file-level). Build-time-only
attributions: `caniuse-lite` (CC-BY-4.0), `argparse` (Python-2.0).

Reproduce the inventories:
```bash
cargo metadata --format-version 1 --manifest-path src-tauri/Cargo.toml \
  | python -c "import sys,json;[print(p['name'],p['version'],p.get('license')) for p in json.load(sys.stdin)['packages']]"
pnpm licenses list
```
