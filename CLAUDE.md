# Document Finder — developer & agent guide

The single source of developer truth for this repo. User-facing docs live in
[`README.md`](README.md); this file is everything else — architecture, build
internals, verification, release, conventions, third-party licenses, icon regen.
Written to double as context for coding agents.

Document Finder is a Tauri 2 desktop app: a **Rust** backend (`src-tauri/`) and a
**Solid.js + Vite + TypeScript** frontend (`src/`). It searches open-access
sources in parallel, downloads documents, extracts clean Markdown + chunks, and
stores everything in per-query SQLite libraries on disk. There are **no AI models
and no user-tunable depth/quality knobs** (removed in 0.3): ranking is model-free
and the pipeline sizes itself. AI access is via export, "Copy for AI", and the
built-in MCP server (`src-tauri/src/mcp.rs`).

## Architecture

### Frontend (`src/`)
- `main.tsx` — entry: applies the theme, registers Tauri event listeners
  (`listenAll` → `runStore.apply`), syncs the library root. In `pnpm dev` without
  Tauri it installs `src/dev/mock.ts` (mock backend for design review; dev-only).
- `App.tsx` — shell: `Sidebar` + one of `Discover` / `LibraryView` /
  `SettingsView`, plus `Overlays` (confirm dialog + toast) and the SR announcer.
- `components/` — `Discover` (hero query, source picker, live run view: three
  stages Search/Download/Read, in-progress list, the "filed" ledger, failure and
  details disclosures; also exports `copyLibraryForAi` / `exportLibrary`),
  `LibraryView` (library list + global full-text search, document list with
  passage search, inspector with metadata + Markdown preview), `SettingsView`
  (appearance, library folder, Connect your AI / MCP, diagnostics, erase data,
  about), `Sidebar`, `Note`, `Overlays`, `Logo`.
- `stores/` — `run` (live run reducer + `stages()` / `progressFraction()`),
  `ui` (view routing, toasts, in-app `confirm()`), `settings` (library root +
  selected sources only), `theme` (`system` / `light` / `dark`).
- `lib/` — `tauri.ts` (typed `invoke` bindings), `events.ts` (event payloads;
  mirror of `src-tauri/src/events.rs`), `utils.ts`, `errors.ts`.
- `styles/globals.css` — the design system (see `.planning/design-contract.md`):
  Instrument Serif + Archivo bundled via `@fontsource` (the CSP blocks CDNs),
  `[data-theme=light|dark]` tokens, one accent, `df-*` classes. Never use
  template-literal class ternaries (see Conventions).

### Backend (`src-tauri/src/`)
- `lib.rs` — Tauri builder, `invoke_handler!` (the command allowlist), setup
  hook (in-process SearXNG, PDFium resource dir). `--mcp` as the first CLI arg
  runs the MCP stdio server instead of the app. `APP_IDENTIFIER` lives here.
- `commands.rs` — every `#[tauri::command]`. Must stay in sync with
  `permissions/app.toml` (enforced by the `permissions_in_sync` test).
- `events.rs` — event-name constants, payload structs, `classify_source_error`,
  and the `Sink` trait (the pipeline emits through it, so it runs headless in
  tests and `examples/bench.rs`).
- `mcp.rs` — read-only MCP server (JSON-RPC over stdio): `list_libraries`,
  `search_library`, `list_documents`, `get_document`, `get_passages`. Finds the
  library root via a file the app writes on `set_library_root`.
- `sources/` — `trait Source` (`mod.rs`) with one module per backend:
  `arxiv`, `openalex`, `semantic_scholar`, `europe_pmc`, `internet_archive`,
  `doaj`, `zenodo`, `gutenberg`, and the web layer: `meta_search` over six HTML
  scrapers with `searxng_pool` and `local_searxng` as fallbacks. `Document`
  carries `doi`, `alt_urls` (fallback download URLs), `cited_by`, `venue`,
  `license`; `best_doi()` / `all_urls()` are the accessors to use.
- `engine/`:
  - `orchestrator` — the streaming pipeline (module doc has the full story).
    Wave 1 discovery → a dispatcher re-ranks every second and feeds a
    best-first queue → download workers start within seconds → pseudo-relevance
    feedback wave 2 + bounded citation graph → single SQLite writer thread.
    Budgets are internal constants (`MAX_CANDIDATES`, `MAX_DOWNLOADS`, …); there
    is no user-facing depth.
  - `ranking` — BM25F + concept coverage + phrase bonus + RRF agreement +
    authority + citations; `expansion_terms` / `feedback_subqueries` (RM3).
  - `dedup` — lossless merge (URLs become fallbacks; DOI/abstract/metadata
    filled in).
  - `downloader` — route chain (all URLs → Unpaywall OA locations), landing
    page → PDF resolution (2 hops), known-repository rewrites
    (`canonicalize_doc_url`), cancel-aware retries honouring `Retry-After`.
  - `hostlimit` — per-host AIMD concurrency.
  - `extract` (PDFium → `pdf-extract` fallback, EPUB spine order, HTML, TXT
    encoding detection), `textclean` (headers/footers, hyphenation, headings,
    page markers, chunking), `html2md`.
  - `db` — schema + `PRAGMA user_version` migrations (v2 added metadata
    columns, `chunks` + `chunks_fts` FTS5), `LibraryIndex` (library-wide
    url/sha256 → path, for cross-library reuse).
  - `library` — read side shared by commands + MCP: list/search/read, citations,
    BibTeX, `build_corpus`, `write_ai_pack`.
  - `query`, `authority`, `citation_graph`, `manifest` (legacy read path), `runlog`.
- `util/` — `path_safety` (confine library paths) and `url_safety`
  (`validate_url`: https-only, no credentials, public-IP-only SSRF check, shared
  DNS cache with `PublicOnlyResolver`).

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
pnpm tauri dev
pnpm tauri build                 # native installers
pnpm clean-build[:full|:dev]     # cross-OS clean rebuild (scripts/rebuild.mjs)
```

- **PDFium**: the PDF engine is a native library bundled per platform from
  `src-tauri/resources/pdfium/` (git-ignored). `scripts/fetch-pdfium.mjs`
  (`pnpm fetch-pdfium`) downloads the pinned build from bblanchon/pdfium-binaries
  (`chromium/8125`, matching `pdfium-render`'s `pdfium_latest` feature — bump
  both together). Tauri's before-dev/build commands run it automatically, but
  **plain `cargo build/test/clippy` fails in `build.rs` until it has been run
  once** (tauri-build checks bundle resources exist). macOS bundles it into
  `Contents/Frameworks` (`tauri.macos.conf.json`); Linux/Windows as resources
  (`tauri.{linux,windows}.conf.json`). At runtime `extract::pdfium()` searches
  the resource dir, the exe dir, `../Frameworks`, and `PDFIUM_DYNAMIC_LIB_PATH`,
  falling back to `pdf-extract` if it can't load.
- **UI without Tauri**: `pnpm dev` and open http://localhost:1420 — the dev mock
  backend (`src/dev/mock.ts`) simulates libraries and a live run.
- **Benchmark**: `cargo run --release --example bench -- "query" …` runs the real
  pipeline headless against the live sources and prints time-to-first-doc,
  saved/failed, text %, throughput. `--example extract -- file.pdf` prints the
  extracted Markdown + stats.
- **pnpm 11 / esbuild**: `pnpm-workspace.yaml` sets `allowBuilds: { esbuild: false }`;
  installs pass `--ignore-scripts`.
- **Linux system deps** (mirrors `ci.yml`/`release.yml`): `libwebkit2gtk-4.1-dev
  libgtk-3-dev libglib2.0-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
  libappindicator3-dev librsvg2-dev patchelf`. On Pop!_OS (Ayatana fork) use
  `libayatana-appindicator3-dev` instead of `libappindicator3-dev`.

## Verify (mirrors CI)

Frontend:
```bash
pnpm lint          # eslint, max-warnings 15
pnpm format:check  # prettier
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest
pnpm build         # vite
```

Backend (run `pnpm fetch-pdfium` once first):
```bash
cargo fmt   --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml
```

`ci.yml` runs all of the above on every PR to `main`; the full 3-OS Tauri build
matrix runs on push-to-`main` or a `full-build`-labelled PR. `release.yml` builds
installers on a `v*` tag (Linux on **ubuntu-22.04** → glibc 2.35+ packages).

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
- **Linux identifiers**: `document-finder` (run-log/state dir), the Tauri
  `identifier` in `tauri.conf.json`, and the Flatpak app id in
  `packaging/flatpak/*.yml` are each also hardcoded in `scripts/uninstall.sh` —
  the `linux_identifiers_in_sync` tests fail otherwise.

## Release

1. Bump the version in **all three**: `package.json`, `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json`.
   ```bash
   git commit -am "chore: bump version to x.y.z"
   git tag vx.y.z
   git push origin main --tags
   ```
2. A `create-release` job opens **one** draft Release for the tag, then
   `release.yml` builds installers into it via tauri-action's `releaseId` —
   macOS (Apple Silicon `.dmg`, **Developer ID signed + notarized**), Linux
   (`.deb`, `.rpm`, `.AppImage`, plus a `.flatpak` from `packaging/flatpak/`),
   Windows (`.exe` + `.msi`). Review, edit notes, publish.

### macOS notarization

The macOS leg signs with a Developer ID and notarizes when these **repository
secrets** exist (Settings → Secrets and variables → Actions); otherwise it falls
back to ad-hoc signing with a workflow warning:

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the *Developer ID Application* certificate exported as `.p12` (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Web World Wide (TEAMID1234)` |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID (Users and Access → Integrations → Keys) |
| `APPLE_API_KEY` | the API key ID |
| `APPLE_API_PRIVATE_KEY` | contents of the downloaded `AuthKey_<id>.p8` |
| `APPLE_TEAM_ID` | 10-character team ID |

Requires a paid Apple Developer Program membership. The app uses the hardened
runtime with `src-tauri/Entitlements.plist` (network client +
`disable-library-validation` so the bundled PDFium dylib loads). Local builds are
unsigned unless you export the same variables.

The Flatpak builds against the **GNOME runtime** (`org.gnome.Platform`/`Sdk`),
not the bare freedesktop runtime, because only the GNOME runtime ships
`libwebkit2gtk-4.1` (a Tauri Linux app dlopen's it). The flatpak job runs an
`ldd` smoke check that fails the release if the chosen runtime is missing the lib
— bump `runtime-version` (manifest) and the `flatpak install` line together. It
also blocking-validates the patched `.desktop` file (`desktop-file-validate`) and
the AppStream metainfo (`appstreamcli validate --pedantic`,
`packaging/flatpak/com.webworldwide.DocumentFinder.metainfo.xml`), plus runs
`flatpak-builder-lint` (the real Flathub linter, via the `org.flatpak.Builder`
Flatpak — not `pip install`) as a non-blocking advisory check, since some of its
checks (e.g. a Flathub-hosted screenshot) can't pass before an actual Flathub
submission. File dialogs use `tauri-plugin-dialog`'s `xdg-portal` feature on
Linux (real `org.freedesktop.portal.FileChooser`, not the raw GTK3 chooser);
`--filesystem=home` stays broad regardless, since the custom-library-root
feature needs direct (non-portal) filesystem tree access.

## Uninstall / data purge

Clean uninstall is three layers (no native uninstaller can remove the user's
document library, and shouldn't auto-nuke `~/Documents`):
- **`purge_all_data` command** (`commands.rs`) — the in-app "Settings → Danger
  zone → Erase app data". It takes **no path argument** by design: it only
  deletes dirs derived server-side from the app identifier (`app_data_dir()` +
  `app_local_data_dir()` + `app_config_dir()` + `app_cache_dir()` — config,
  legacy 0.2 model files, and whatever the webview itself caches),
  `runlog::log_path()`'s parent (the per-OS log dir), and — only when
  `include_library` is set — `confinement_root()` (which knows a custom library
  root). It reuses `force_remove_dir` and is best-effort (returns a
  `PurgeReport { removed, failed }`). Registered in the usual four places
  (`commands.rs`, `lib.rs`, `permissions/app.toml`, `src/lib/tauri.ts`).
- **`scripts/uninstall.{ps1,sh}`** — standalone per-user data wipe for when the
  app is already gone. They only know the **default** `~/Documents/Document
  Finder`; keep their hard-coded identifier (`com.webworldwide.documentfinder`)
  and log paths in sync with `runlog.rs` if those ever change.
- **NSIS hook is intentionally deferred** — Tauri's generated Windows uninstaller
  already ships an opt-in "Delete application data" checkbox covering
  `%APPDATA%`/`%LOCALAPPDATA%`; a force hook would only remove that choice and
  still must not touch the Documents library.

## Icon regeneration

Master artwork lives in `icons/` (not shipped at runtime, with one deliberate
exception: `packaging/flatpak/com.webworldwide.DocumentFinder.yml` pulls
`Document Finder Icon.svg` directly into the Flatpak build as its scalable
`hicolor/scalable/apps/` icon, since Tauri's bundler only emits raster sizes).
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
**Settings → Pages → Source: GitHub Actions**. It shares the app's design system
(Instrument Serif + Archivo self-hosted in `site/fonts/`, the same tokens) and
shows real product screenshots (`site/assets/`, captured from the dev mock UI;
re-capture after visual changes). `site/og.png` (1200×630) is the link preview.
Served at the custom domain **documentfinder.webworldwide.online** via
`site/CNAME` (included in the uploaded artifact, so it persists across deploys);
a DNS `CNAME documentfinder → webworldwide.github.io` record points to it. Keep the
canonical/`og:`/`twitter:` URLs in `site/index.html` on that domain.

## Third-party licenses

Document Finder is licensed under **GNU AGPL-3.0-or-later** (see [`LICENSE`](LICENSE)),
copyright Web World Wide. The third-party components below are informational and do
not modify any component's terms. License inventory last audited **2026-05-30**.

**Bundled native library.** PDFium (BSD-3-Clause, Google/Chromium) via the
bblanchon/pdfium-binaries builds; its license ships next to it
(`pdfium/PDFIUM-LICENSE.txt`). **Fonts**: Instrument Serif and Archivo (SIL OFL
1.1), bundled in the app via `@fontsource` and in `site/fonts/` with their
licenses. No AI model weights are downloaded or bundled (removed in 0.3).

**Rust dependencies** are all permissive / AGPL-compatible — predominantly
`MIT OR Apache-2.0`, with assorted MIT/BSD/Zlib/ISC/Unicode-3.0 and a few MPL-2.0
(file-level). No GPL-2.0-only/AGPL/SSPL/BUSL/CC-BY-NC/proprietary deps. Notable
native/parsing libs (all permissive): `pdfium-render` (MIT/Apache-2.0),
`pdf-extract` (MIT), `scraper` (ISC), `rusqlite`/bundled SQLite (public domain /
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
