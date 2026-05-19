<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Document Finder Logo" width="160" height="160" />
  <h1>Document Finder v2</h1>
  <p><strong>A blazingly fast, cross-platform desktop application for discovering, downloading, and bundling open-access research for AI contexts.</strong></p>
</div>

---

Document Finder v2 is a complete rewrite of the original Python-based tool, now powered by **Tauri 2**, a high-performance **Rust** backend, and a **Solid.js/Vite** frontend. It enables you to concurrently search multiple open-access platforms, download documents, extract text, and instantly package them into clean, AI-ready datasets.

## Key Features

- **Unified Discovery**: Search across multiple open-access sources simultaneously. Natural-language query expansion splits your query into sub-queries automatically.
- **Live Download Stream**: Watch documents stream in as they are fetched and processed by the asynchronous Rust backend with exponential backoff and silent retries.
- **Library Management**: Manage your collections in the Library tab. View metadata, doc counts, and total sizes.
- **AI-Ready Exports**: Export libraries as `.zip` files containing PDFs, EPUBs, and extracted plain text — ready to drop into any AI context window.
- **Blazing Fast**: Rust handles parallel downloads, PDF/EPUB text extraction, and SQLite persistence natively.
- **Privacy-First Meta-Search**: Aggregates DuckDuckGo, Bing, Brave, and public SearXNG instances with a circuit breaker — no Docker required, no setup.
- **Local AI Ranking**: Two small bundled models — `bge-small-en-v1.5` (~33 MB) for semantic reranking and `Qwen 2.5 3B Instruct` (~2 GB) for query expansion + borderline filtering. Manage downloads from **Settings → AI Models**. Everything runs offline; no API keys, ever.
- **Editorial Design System**: Three themes (Paper / Slate / Dark) + nine accent colors + density toggle, switchable from Settings.
- **Embedded SearXNG-Compatible Server**: An `axum`-backed local search service runs in-process on a random localhost port — Docker-free, Python-free, ready immediately at app start.

## Supported Sources

*No API keys required. All sources are open-access.*

| Source | Description |
|--------|-------------|
| [arXiv](https://arxiv.org/) | Preprints in CS, physics, math, and more |
| [OpenAlex](https://openalex.org/) | ~250M scholarly works with open-access filter |
| [Semantic Scholar](https://www.semanticscholar.org/) | ~200M papers with PDF links |
| [Internet Archive](https://archive.org/) | Millions of books, papers, and media |
| [DOAJ](https://doaj.org/) | Directory of Open Access Journals |
| [Project Gutenberg](https://www.gutenberg.org/) | 70,000+ free ebooks |
| **Web** | Meta-search aggregator: DuckDuckGo, Bing, Brave + public SearXNG pool |

---

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (installed via `rustup`)
- [Node.js](https://nodejs.org/) v20+
- **C++ build toolchain** (for the local LLM via llama.cpp):
  - **macOS**: `brew install cmake` + Xcode Command Line Tools (`xcode-select --install`). Metal GPU support is built in.
  - **Linux**: cmake + clang/g++ (e.g. `sudo apt install build-essential cmake clang`).
  - **Windows**: cmake + Visual Studio Build Tools 2022 with the C++ workload.

> **pnpm** is installed automatically by `run.sh` if not present.

> The first `cargo build` is slow (10–20 min) because llama.cpp + ONNX
> Runtime get compiled from source. Subsequent builds are cached.
>
> To skip the AI features entirely (faster builds, no semantic
> reranking or LLM expansion), build with:
> `cargo build --no-default-features --features=custom-protocol`

### One-Click Launch

```bash
# macOS and Linux
./run.sh

# Windows (PowerShell)
.\run.ps1
```

`run.sh` detects whether it's running on macOS or Linux and branches:
- **macOS**: builds the `.app`, installs to `/Applications/`, launches it.
- **Linux**: builds the binary, launches `src-tauri/target/release-fast/document-finder` directly. Logs to `/tmp/document-finder.log`.

`run.ps1` (Windows) builds an MSI, copies the binary + DLLs to `%LOCALAPPDATA%\Programs\Document Finder\`, and launches it. For a real Start-menu install, double-click the `.msi` at `src-tauri\target\release-fast\bundle\msi\`.

Each script checks prerequisites and installs missing dependencies. First Rust build is slow (10-20 min) because llama.cpp + ONNX Runtime compile from source; subsequent builds are cached.

### Manual Setup

```bash
# Install Node dependencies
pnpm install

# Start development server
pnpm tauri dev
```

### Build Native Installer

```bash
pnpm tauri build
```

This produces platform-native installers in `src-tauri/target/release/bundle/`.

---

## Data Storage

Each search creates a folder under your configured library root:

```
~/Documents/DocumentFinder/
└── your-query-slug/
    ├── library.db        ← SQLite database (metadata, run history)
    ├── _text/            ← Extracted plain text files
    ├── paper-title-abc123.pdf
    └── ...
```

The `library.db` file contains full metadata for all downloaded documents and can be queried directly with any SQLite client.

---

## Tech Stack

| Layer | Technology |
|-------|------------|
| Desktop shell | [Tauri 2](https://tauri.app/) |
| Backend | Rust (tokio async runtime) |
| Frontend | [Solid.js](https://www.solidjs.com/) + TypeScript |
| Bundler | [Vite 6](https://vitejs.dev/) |
| Styling | Plain CSS — design tokens flip via `data-theme` / `data-accent` / `data-density` on `<body>` |
| Embedded search | [axum](https://docs.rs/axum/) — SearXNG-compatible local HTTP server |
| Database | SQLite via rusqlite (bundled) |

---

## Contributing

Contributions welcome. The Rust sources live in `src-tauri/src/sources/` — each source is a self-contained module implementing the `Source` trait with a `search()` method that returns a stream of `Document`s.
