<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Document Finder" width="140" height="140" />
  <h1>Document Finder</h1>
  <p><strong>Find open-access research across the web, download it in bulk, and bundle it into a tidy local library — ready to drop into any AI context window.</strong></p>
  <p>
    <a href="https://adamnolle.github.io/Document-Finder/">Website</a> ·
    <a href="https://github.com/AdamNolle/Document-Finder/releases">Download</a> ·
    <a href="CLAUDE.md">Developer guide</a>
  </p>
</div>

---

Document Finder started as a way to **find and compress documents into context for AI and RAG**, and grew into a broader tool for discovering and downloading open-access research. It searches several sources in parallel, downloads the papers, extracts their text, and stores everything in per-query SQLite libraries you can export as a single `.zip`. Native desktop app — **Rust** backend, **Solid.js** frontend, **Tauri**.

## Features

- **Unified discovery** across seven open-access sources, with natural-language query expansion into sub-queries.
- **Live download stream** — watch documents arrive in real time with throughput, ETA, file-type breakdown, and a per-source lane chart as the async Rust backend fetches, retries, and extracts text.
- **Plain-language results** — every skipped or failed download explains itself in one sentence ("This source blocked the download — it may need a sign-in"), not an HTTP code.
- **AI-ready exports** — bundle any library (PDFs, EPUBs, extracted text) into a `.zip` for a context window.
- **Built-in web meta-search** — DuckDuckGo, Bing, Brave, Mojeek, Marginalia, and Startpage behind a per-engine circuit breaker, with an in-process SearXNG-compatible server and a public SearXNG pool as fallbacks. No Docker, no setup, no API keys.
- **Optional on-device AI** — `bge-small` reranks results and an on-device `Qwen 2.5 1.5B` expands queries and filters borderline hits; both downloaded on first use, run fully offline.
- **Editorial workstation UI** — three themes (Paper / Slate / Midnight), nine accent colors, compact/regular density, and a stacked-or-split download stream, with reduced-motion support. Defaults to Slate + Sky blue.

## Sources

_All open-access. No API keys._

| Source | What it covers |
| --- | --- |
| [arXiv](https://arxiv.org/) | Preprints in CS, physics, math, and more |
| [OpenAlex](https://openalex.org/) | ~250M scholarly works, open-access filtered |
| [Semantic Scholar](https://www.semanticscholar.org/) | ~200M papers with PDF links |
| [Internet Archive](https://archive.org/) | Books, papers, and scanned media |
| [DOAJ](https://doaj.org/) | Directory of Open Access Journals |
| [Project Gutenberg](https://www.gutenberg.org/) | 70,000+ public-domain ebooks |
| **Web** | Built-in meta-search (DuckDuckGo, Bing, Brave, Mojeek, Marginalia, Startpage) + SearXNG fallback |

## Installing a release

The [release builds](https://github.com/AdamNolle/Document-Finder/releases) are
unsigned (the macOS build is ad-hoc signed), so each OS asks you to allow the app
on first launch:

- **macOS** (Apple Silicon) — right-click the app → **Open**, then confirm. If
  macOS still says the app *"is damaged and can't be opened"*, clear the download
  quarantine flag and reopen:
  ```bash
  xattr -dr com.apple.quarantine "/Applications/Document Finder.app"
  ```
- **Windows** — SmartScreen → **More info** → **Run anyway**.
- **Linux** — the `.deb`/`.rpm`/AppImage need glibc 2.39+ (built on Ubuntu 24.04);
  the Flatpak ships its own runtime, so it runs anywhere Flatpak does.

## Uninstalling

Document Finder is easy to remove cleanly on any OS.

1. **Erase its data (recommended, any OS).** In the app: **Settings → Danger
   zone → Erase app data**. This deletes the downloaded AI models, caches, and
   run logs; tick the extra box to also delete your downloaded document library.
   This is the only step that knows a **custom** library folder, so do it first
   if you moved your library. Then quit the app.
2. **Remove the app itself:**
   - **Windows** — Settings → Apps → **Document Finder** → Uninstall. The
     uninstaller offers a *"Delete application data"* checkbox (clears the
     `%APPDATA%`/`%LOCALAPPDATA%` caches incl. WebView2 storage); it never
     touches your `Documents\Document Finder` library.
   - **macOS** — quit, then drag **Document Finder.app** to the Trash.
   - **Linux** — `sudo apt purge document-finder` (`.deb`), `sudo dnf remove
     document-finder` (`.rpm`), delete the `.AppImage`, or `flatpak uninstall
     --delete-data com.webworldwide.DocumentFinder`.

Prefer the command line, or already deleted the app? Run the bundled script to
clear all per-user data (it prompts before touching your library):

```bash
scripts/uninstall.sh          # macOS / Linux
scripts/uninstall.ps1         # Windows (PowerShell)
```

<details><summary>Exact data locations removed</summary>

| What | Windows | macOS | Linux |
| --- | --- | --- | --- |
| AI models + caches + config | `%APPDATA%\com.webworldwide.documentfinder` | `~/Library/Application Support/com.webworldwide.documentfinder` | `~/.local/share/com.webworldwide.documentfinder` |
| Webview storage / caches | `%LOCALAPPDATA%\com.webworldwide.documentfinder` | `~/Library/{WebKit,Caches,Preferences}/com.webworldwide.documentfinder*` | `~/.config` + `~/.cache/com.webworldwide.documentfinder` |
| Run log | `%LOCALAPPDATA%\Document Finder\Logs` | `~/Library/Logs/Document Finder` | `~/.local/state/document-finder` |
| Document library (kept by default) | `Documents\Document Finder` | `~/Documents/Document Finder` | `~/Documents/Document Finder` |

</details>

## Build and run

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 22+, and a C++ toolchain with **cmake** + **clang/LLVM** (for the bundled llama.cpp). On Windows, install [LLVM](https://github.com/llvm/llvm-project/releases) (or use Visual Studio Build Tools with the C++ and Clang components) so `libclang` is on your `PATH`.

```bash
./run.ps1         # Windows  (./run.sh on macOS/Linux — installs pnpm if needed)
pnpm tauri build  # native installers in src-tauri/target/release/bundle/
```

The first build compiles llama.cpp + ONNX Runtime from source (10–25 min). For a fast compile-check without the AI features (note: `pnpm tauri build` does not forward `--no-default-features` to cargo, so call cargo directly):

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features=custom-protocol
```

To rebuild cleanly later — one command on any OS (Windows, macOS, Linux):

```bash
pnpm clean-build         # clean caches + release-fast build
pnpm clean-build:dev     # clean + hot-reload dev build
```

## Data

Each search gets a folder under `~/Documents/Document Finder/library/`:

```
your-query-slug/
├── library.db                ← SQLite metadata
├── _text/                    ← extracted plain text
└── paper-title-abc123.pdf    ← the downloaded files
```

`library.db` holds full metadata and is queryable with any SQLite client.

## Contributing and license

See [`CLAUDE.md`](CLAUDE.md) for the architecture map, build internals, release process, third-party license notices, and icon regeneration. New sources live in `src-tauri/src/sources/` as modules implementing the `Source` trait.

Licensed under the [GNU Affero General Public License v3.0](LICENSE). Copyright © 2026 Web World Wide.
