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
- **Optional on-device AI** — `bge-small` reranks results and a bundled `Qwen 2.5 1.5B` expands queries and filters borderline hits; downloaded on first use, run fully offline.
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

## Build and run

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 22+, and a C++ toolchain with **cmake** + **clang/LLVM** (for the bundled llama.cpp). On Windows, install [LLVM](https://github.com/llvm/llvm-project/releases) (or use Visual Studio Build Tools with the C++ and Clang components) so `libclang` is on your `PATH`.

```bash
./run.ps1         # Windows  (./run.sh on macOS/Linux — installs pnpm if needed)
pnpm tauri build  # native installers in src-tauri/target/release/bundle/
```

The first build compiles llama.cpp + ONNX Runtime from source (10–25 min). For fast dev builds without the AI features:

```bash
pnpm tauri build --no-default-features --features custom-protocol
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
