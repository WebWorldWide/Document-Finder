<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Document Finder Logo" width="128" height="128" />
  <h1>Document Finder v2</h1>
  <p><strong>A blazingly fast, cross-platform desktop application for discovering, downloading, and bundling open-access research for AI contexts.</strong></p>
</div>

---

Document Finder v2 is a complete rewrite of the original Python-based tool, now powered by **Tauri 2**, a high-performance **Rust** backend, and a modern **React/Vite** frontend. It enables you to concurrently search multiple open-access platforms, download documents, extract text, and instantly package them into clean, AI-ready datasets.

## Key Features

- **Unified Discovery (Find Tab)**: Search across a multitude of open-access sources simultaneously. Split sub-queries easily using natural language (e.g., using commas, "and", or "&").
- **Live Download Stream**: Watch documents stream in as they are fetched and processed by the asynchronous Rust backend with exponential backoff and silent retries for high reliability.
- **Library Management**: Manage your gathered collections in the Library tab. View metadata, extracted texts, and total dataset sizes with zero-latency background scanning.
- **Streaming AI-Ready Exports**: Export your libraries as consolidated `.zip` files. Uses high-performance streaming I/O to handle massive datasets without memory overhead.
- **Enterprise-Grade Security**: Built with strict Content Security Policy (CSP) and granular filesystem scopes to ensure your local data remains protected.
- **Blazing Fast & Lightweight**: The Rust backend handles parallel downloads, PDF/EPUB extraction, and file management natively with incredibly low resource usage.

## Supported Sources
*No API keys required. All sources are open-access.*

- [arXiv](https://arxiv.org/)
- [OpenAlex](https://openalex.org/)
- [Semantic Scholar](https://www.semanticscholar.org/)
- [Internet Archive](https://archive.org/)
- [Directory of Open Access Journals (DOAJ)](https://doaj.org/)
- [Project Gutenberg](https://www.gutenberg.org/)
- **DuckDuckGo** *(via permissive document discovery)*

---

## Getting Started

### Prerequisites
- [Rust](https://rustup.rs/) (`rustup-init`)
- [Node.js](https://nodejs.org/) (v20+)
- [pnpm](https://pnpm.io/)

### One-Click Launch
For convenience, you can run the following script from the root directory. It will check for prerequisites, install dependencies, and launch the app:

```bash
./run.sh
```

### Running from Source manually
Clone the repository and run the development environment:

```bash
# Install Node dependencies
pnpm install

# Start the Tauri development server
pnpm tauri dev
```

### Building Native Installers

You can compile native standalone installers for your specific platform using Tauri:

```bash
# Build for your current platform (macOS/Windows/Linux)
cargo tauri build                                      

# Cross-build for Windows (if configured)
cargo tauri build --target x86_64-pc-windows-msvc      

# Cross-build for Linux (if configured)
cargo tauri build --target x86_64-unknown-linux-gnu    
```

Compiled binaries and installers will be generated under `src-tauri/target/release/bundle/`.

---

## Data Architecture

Document Finder organizes downloads into isolated library folders. 

```text
~/Documents/Document Finder/<query-slug>/
├── manifest.json          # Metadata and provenance for all documents
├── _text/                 # Extracted plaintext (.txt) for each document
└── [original files]       # Source documents (PDF, EPUB, HTML, etc.)
```

### Security & Isolation
Document Finder v2 adheres to the principle of least privilege. Filesystem access is strictly scoped to the `Documents/Document Finder` directory. All network requests are governed by a strict Content Security Policy to prevent unauthorized data exfiltration or script execution.

> **Note on Backwards Compatibility:** The `bundle.json.gz` schema is identical to the v1 Python app. Libraries created with v1 will seamlessly open in v2 without needing to be re-bundled.

## License & Ethical Usage

All integrated sources are open-access by policy. This application **does not** bypass paywalls, implement DRM circumvention, or scrape copyrighted, restricted material. Please respect the rate limits and terms of service of the respective indexing platforms. 
