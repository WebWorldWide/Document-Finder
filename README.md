<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Document Finder" width="120" height="120" />
  <h1>Document Finder</h1>
  <p>A desktop app for discovering, downloading, and bundling open-access research into AI-ready libraries.</p>
</div>

---

Search arXiv, OpenAlex, Semantic Scholar, the Internet Archive, DOAJ, Project Gutenberg, and an embedded web metasearch in parallel. Document Finder downloads PDFs/EPUBs/HTML, extracts plain text, and persists every run to a queryable SQLite library — all locally, with no API keys.

## Install

Download an installer from the [latest release](https://github.com/AdamNolle/Document-Finder/releases):

| Platform | File | First-launch |
|---|---|---|
| macOS (Apple Silicon) | `.dmg` | Drag to Applications. Gatekeeper blocks unsigned apps on first run — System Settings → Privacy & Security → **Open Anyway**. |
| Linux | `.deb` or `.AppImage` | `.deb`: `sudo dpkg -i …`. `.AppImage`: `chmod +x` then run. |
| Windows | `.msi` or NSIS `.exe` | SmartScreen will warn — **More info** → **Run anyway**. |

Binaries are unsigned (no Developer cert). The app makes no outbound calls except to the open-access sources you enable; verify with `codesign -dvvv` (macOS) or `Get-AuthenticodeSignature` (Windows).

## Features

- **Parallel discovery** across all enabled sources with automatic query expansion into sub-queries.
- **Live download stream** with throughput sparkline, ETA, sub-query progress, and per-source lane chart.
- **Local AI ranking** (optional, downloaded on demand from Settings → AI Models):
  - `bge-small-en-v1.5` (~33 MB) for semantic re-rank.
  - `Qwen 2.5 3B Instruct` (~2 GB) for query expansion + borderline filtering.
- **Embedded SearXNG-compatible server** runs in-process on a localhost port — no Docker, no Python, no setup.
- **Library management** — view collections, export as ZIP, delete from disk, see lifetime per-source stats.
- **Editorial theme system** — Paper / Slate / Dark + nine accents, persisted across launches.
- **Structured logs** in Settings → Logs for bug-report exports.

## Sources

| Source | Coverage |
|---|---|
| [arXiv](https://arxiv.org/) | Preprints in CS, physics, math |
| [OpenAlex](https://openalex.org/) | ~250M scholarly works |
| [Semantic Scholar](https://www.semanticscholar.org/) | AI-augmented paper index |
| [Internet Archive](https://archive.org/) | Books, papers, scans |
| [DOAJ](https://doaj.org/) | Open-access journals |
| [Project Gutenberg](https://www.gutenberg.org/) | Public-domain books |
| Web | Embedded metasearch — DuckDuckGo, Bing, Brave |

## Build from source

Prerequisites: [Rust](https://rustup.rs/), [Node.js 22+](https://nodejs.org/), cmake + a C++ toolchain (Xcode CLI tools on macOS, `build-essential` on Linux, Visual Studio Build Tools on Windows).

```bash
# macOS / Linux
./run.sh

# Windows (PowerShell)
.\run.ps1
```

First Rust build takes 10–20 minutes (llama.cpp and ONNX Runtime compile from source). For a development loop, use `pnpm tauri dev` instead — debug profile with hot reload.

## Data layout

```
~/Documents/DocumentFinder/
└── <query-slug>/
    ├── library.db        SQLite — metadata, run history, full-text index
    ├── _text/            extracted plain text
    └── <paper>.pdf
```

`library.db` is a standard SQLite file. Inspect with any SQLite client.

## Tech stack

Tauri 2 · Rust (tokio) · Solid.js · Vite · SQLite · axum (embedded search server)
