<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Document Finder" width="140" height="140" />
  <h1>Document Finder</h1>
  <p><strong>Describe a topic. Document Finder searches open-access libraries in parallel, downloads the best matches, and turns them into clean, cited text for your AI.</strong></p>
  <p>
    <a href="https://documentfinder.webworldwide.online/">Website</a> ·
    <a href="https://github.com/WebWorldWide/Document-Finder/releases">Download</a> ·
    <a href="CLAUDE.md">Developer guide</a>
  </p>
</div>

---

Document Finder is a native desktop app (Rust + Tauri, Solid.js) that turns a plain-language question into a local research library. It searches eight open-access scholarly sources plus the web at once, ranks and de-duplicates the results, downloads the best matches, and extracts clean Markdown you can search, cite, and hand to any AI — by copy-paste, as an export pack, or live over MCP. No accounts, no API keys, no AI model downloads, and nothing to tune.

## Features

- **Just works** — no depth, quality, or concurrency settings. The app decides how far to search (it keeps going while new results stay relevant) and how fast to download (a per-site speed limit that adapts itself and backs off when a server says "slow down").
- **Fast** — downloads start within seconds, best match first, while searching continues. HTTP/2, a shared DNS cache, and reuse of anything already in your library.
- **Finds the open copy** — every known open-access location for a paper is tried: publisher, repository, preprint, Europe PMC, bioRxiv, OSF, and Unpaywall for every DOI. Duplicate results from different sources are merged, so no information is lost.
- **Model-free relevance** — BM25F ranking with concept coverage, cross-source agreement, citation counts, and a citation-graph boost, plus a second search pass that learns the field's own vocabulary from the best first results (pseudo-relevance feedback).
- **Clean text** — PDFs are read with PDFium (Chrome's PDF engine). Running headers, page numbers, ligatures, and line-break hyphenation are removed; sections, page markers, and metadata are kept. EPUBs are read in true reading order, web pages are reduced to their main content, and scanned PDFs are flagged clearly.
- **Your library, searchable** — full-text search (SQLite FTS5) across every library, with an inspector that shows metadata, abstract, citation, and a reading preview.
- **For any AI** — *Copy for AI* (a citation-headed corpus sized for a context window), an *export pack* (`README.md` index, Markdown per document, `chunks.jsonl`, `references.bib`, `corpus.md`), and a built-in **MCP server** so Claude Desktop, Claude Code, or any MCP client can search and read your libraries directly.
- **Plain-language results** — every failed download explains itself in one sentence.
- **Quiet design** — Instrument Serif + Archivo, light / dark / system, keyboard-friendly, reduced-motion aware.

## Sources

_All open-access. No API keys._

| Source | What it covers |
| --- | --- |
| [arXiv](https://arxiv.org/) | Preprints in CS, physics, math, and more |
| [OpenAlex](https://openalex.org/) | ~250M scholarly works, open-access filtered |
| [Semantic Scholar](https://www.semanticscholar.org/) | ~200M papers with PDF links |
| [Europe PMC](https://europepmc.org/) | Life-sciences & biomedical literature |
| [Internet Archive](https://archive.org/) | Books, papers, and scanned media |
| [DOAJ](https://doaj.org/) | Directory of Open Access Journals |
| [Zenodo](https://zenodo.org/) | Open-access papers, datasets & software (CERN) |
| [Project Gutenberg](https://www.gutenberg.org/) | 70,000+ public-domain ebooks |
| **Web** | Built-in meta-search (DuckDuckGo, Bing, Brave, Mojeek, Marginalia, Startpage) + SearXNG fallback |

## Installing a release

Download from the [releases page](https://github.com/WebWorldWide/Document-Finder/releases/latest).

<!-- SYNC: these install notes are duplicated in site/index.html (.dl-fine) and
     .github/workflows/release.yml (create-release notes.md). Update all three. -->
- **macOS** (Apple Silicon, macOS 11+) — the `.dmg` is signed with a Developer ID and notarized by Apple, so it opens normally.
- **Windows** — SmartScreen may ask once: **More info** → **Run anyway**.
- **Linux** — `.deb`, `.rpm`, and `.AppImage` need glibc 2.35+ (Ubuntu 22.04, Debian 12, Fedora 36 or newer); the **Flatpak** runs anywhere. If the window opens blank (some older GPU drivers with the system WebKitGTK), enable **Settings → Diagnostics → Safe rendering mode** and relaunch, or start the app with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

## Connect your AI (MCP)

**Settings → Connect your AI → Add to Claude Desktop** adds Document Finder to Claude Desktop in one click. For Claude Code, copy the command shown there, which looks like:

```bash
claude mcp add document-finder -- "/Applications/Document Finder.app/Contents/MacOS/document-finder" --mcp
```

Any other MCP client can use the JSON config shown in the same panel. The server is read-only and only sees your library folder. It offers `list_libraries`, `search_library`, `list_documents`, `get_document`, and `get_passages`.

## Uninstalling

Document Finder is easy to remove cleanly on any OS.

1. **Erase its data (recommended, any OS).** In the app: **Settings → Danger
   zone → Erase app data**. This deletes settings, caches, and
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
| Config + caches | `%APPDATA%\com.webworldwide.documentfinder` | `~/Library/Application Support/com.webworldwide.documentfinder` | `~/.local/share/com.webworldwide.documentfinder` |
| Webview storage / caches | `%LOCALAPPDATA%\com.webworldwide.documentfinder` | `~/Library/{WebKit,Caches,Preferences}/com.webworldwide.documentfinder*` | `~/.config` + `~/.cache/com.webworldwide.documentfinder` |
| Run log | `%LOCALAPPDATA%\Document Finder\Logs` | `~/Library/Logs/Document Finder` | `~/.local/state/document-finder` |
| Document library (kept by default) | `Documents\Document Finder` | `~/Documents/Document Finder` | `~/Documents/Document Finder` |

</details>

## Build and run

Prerequisites: [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/) 22+ (plus the WebKitGTK dev packages on Linux; see [`CLAUDE.md`](CLAUDE.md)). No C++ toolchain, cmake, or LLVM needed.

```bash
./run.sh          # macOS / Linux (installs pnpm if needed) — ./run.ps1 on Windows
pnpm tauri dev    # hot-reload dev build
pnpm tauri build  # native installers in src-tauri/target/release/bundle/
```

The Tauri commands download the matching PDFium library automatically (`pnpm fetch-pdfium`). Run it once yourself before plain `cargo` commands.

For a clean rebuild on any OS:

```bash
pnpm clean-build         # clean caches + release-fast build
pnpm clean-build:dev     # clean + hot-reload dev build
```

## Data

Each search gets a folder under `~/Documents/Document Finder/library/`:

```text
your-query-slug/
├── library.db                 ← SQLite: metadata, chunks, full-text index
├── _text/paper-title-abc123.md ← clean Markdown with a metadata header
└── paper-title-abc123.pdf     ← the original file
```

`library.db` can be queried with any SQLite client. A small `.document-finder-index.db` in the library root lets later searches reuse files you already have.

## Contributing and license

See [`CLAUDE.md`](CLAUDE.md) for the architecture map, build internals, release process, third-party license notices, and icon regeneration. New sources live in `src-tauri/src/sources/` as modules implementing the `Source` trait.

Licensed under the [GNU Affero General Public License v3.0](LICENSE). Copyright © 2026 Web World Wide.
