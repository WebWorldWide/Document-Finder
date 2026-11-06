# Document Finder 0.3 — overhaul plan

Drafted 2026-11-06 from a full read of the codebase and the `design-ui` skill
(AdamNolle/my-skills). Line refs are as of commit `10fdf2a`.

Goals, from the brief:
1. Redesign the app **and** the site so they look much better (design-ui skill).
2. Better download algorithm + faster downloads, without breaking politeness or stability.
3. Remove the granularity controls — it should just work with no settings.
4. Much better text extraction and output for AI.
5. No local AI model downloads — replace with something that needs no setup.
6. Fix everything else found along the way.

---

## Phase 0 — Safety net and benchmark (do first, ~small)

We can't claim "faster" or "better" without a baseline, and the UI has no tests.

- **`bench/` harness** (Rust integration test behind `--ignored`): 12 fixed queries
  (narrow, broad, multi-topic, misspelled, book-ish, biomedical). Records
  time-to-first-saved-doc, docs saved/min, total wall time, failure rate by reason,
  and on-topic precision@20 (hand-labelled once, stored as JSON). Run before and
  after each backend phase.
- **Frontend test scaffolding**: a mocked `invoke`/`listen` layer so components
  can be rendered in Vitest. Smoke tests for Discover, Library and Settings
  covering empty, running, done and error states. A redesign without this has no net.

---

## Phase 1 — Remove local AI and the knobs (unblocks everything else)

### 1a. Delete the model stack
Remove `src-tauri/src/ai/` and its features (`fastembed`, `llama-cpp-2`,
`encoding_rs`, `sha2`) plus every touchpoint. The full checklist is in the audit:
`lib.rs`, the commands in `commands.rs` and `permissions/app.toml`, the rerank,
LLM-expand and LLM-filter blocks in `orchestrator.rs`, events (Rust and TS together),
`ModelDownloadCard`, `ModelStatusBadge`, `WelcomeDialog`, `stores/models.ts`, and
the quality settings.

Side benefits, all worth having by themselves:
- No cmake, clang or libclang. CI drops the `backend-ai` job and the Windows
  choco LLVM step. Clean builds go from 15–25 min to a few minutes. Smaller installers.
- **Linux reach**: ONNX Runtime was the only reason for the glibc 2.39 floor.
  Building on `ubuntu-22.04` (glibc 2.35) lets the .deb, .rpm and AppImage run on
  Ubuntu 22.04, Debian 12, Mint 21 and similar, and the scary caveat in README,
  the site and the release notes goes away. *Verify the ldd/webkit smoke check on 22.04.*

### 1b. What replaces each AI job (no model, no setup)
| Was | Replacement |
|---|---|
| LLM query expansion (24–40 sub-queries) | **Pseudo-relevance feedback (RM3)**: after the first ~150 results, pull the top-weighted terms from the best titles and abstracts and issue a second wave. Add **OpenAlex topic/keyword lookup** (free, no key) for vocabulary bridging, e.g. "heart attack" → "myocardial infarction". Add a spell-fix from source "did you mean" responses and a small built-in edit-distance check. |
| BGE embedding rerank | **BM25F** (field-weighted title/abstract/venue) replacing ad-hoc TF-IDF, plus RRF cross-source agreement, plus authority, plus OpenAlex `cited_by_count` (log-damped) and `is_oa` quality signals. |
| LLM borderline yes/no filter | Dropped. The calibrated BM25F floor handles obvious junk. Real judgment moves to the user's own AI (Claude, ChatGPT and so on) through the much better export and MCP access in Phase 4. |

**Optional, decide after the benchmark:** bundle a tiny *static* embedding model
(model2vec "potion" family, MIT, roughly 8–30 MB, pure Rust, no ONNX, no download,
runs in microseconds) for a semantic tiebreak. Only ship it if Phase 0 numbers show a
real precision gain over BM25F + RM3.

### 1c. "Just works" — remove granularity
Delete **Download depth** (slider plus Advanced numbers), **Search quality**, the
**citation-graph** toggle, the **cross-run reuse** toggle, the **per-engine web
toggles**, and **stream layout**. `RunRequest` becomes `{ query, sources }`.
Everything else is adaptive in the backend:

- **How much to fetch**: estimate query breadth from first-wave yield. Keep going
  while new results stay on-topic. Stop when the marginal on-topic rate falls
  below a threshold, or at a hard ceiling (around 400 docs, with a time budget).
  Narrow queries finish fast with fewer docs. Broad ones go deeper.
- **Concurrency**: per-host AIMD (start at 4, grow on success, halve on 429/503,
  honor `Retry-After`). There is no global user number.
- **Citation graph**: always on, but bounded (top 30, 10 s budget) and run *in
  parallel* with downloads, so it reorders the queue rather than gating it.
- **Cross-run reuse**: always on, via a content index (see Phase 3).
- **Web engines**: the circuit breaker already handles bad engines. No user toggles.
- **Safe rendering mode (Linux)**: auto-detect a blank first paint and relaunch
  with the WebKit env flags. Keep the checkbox only as a hidden override.
- `start_run` clamps every number server-side. Today it trusts the UI (bug).

---

## Phase 2 — Search and ranking algorithm

1. **Stream ranking into downloads.** Today nothing downloads until discovery,
   rerank, the LLM filter and the citation graph all finish (`orchestrator.rs:1086-1767`).
   New flow: a provisional BM25F score is computed per result as it arrives. A
   priority queue feeds the downloader once a small warm-up set exists (around 40
   results or 3 s). Later, better results jump the queue. Expect a big drop in
   time-to-first-doc.
2. **Fair discovery.** Stop the "first to `max_total` wins" race (`:283, :384`),
   where OpenAlex's 200-per-page fills the set and starves slower, better sources.
   Use per-source quotas plus round-robin interleaving.
3. **Lossless dedup merge** (`dedup.rs:199-282`). Merge all URLs (kept as ordered
   fallbacks), DOI, the longest abstract, authors, and year. Ranking gets abstracts
   for web hits, and downloads get alternate URLs.
4. **Fix keyword counting** (`:202, :1338`). Sub-query terms are counted
   repeatedly and skew TF-IDF. Use the de-duplicated query model instead.
5. **Fix the citation re-sort** (`citation_graph.rs:207-211`), which compares
   normalized and raw scores. One score scale end to end.
6. **Correct rate limits per source.** arXiv gets 1 request per 3 s globally
   (it's 4× today). Internet Archive's inner `buffer_unordered(8)` counts against
   the source budget. meta_search engine tasks become structured children so
   cancel actually stops them.
7. **Request DOIs everywhere.** OpenAlex `select` adds `doi`, `best_oa_location`
   and `locations`, so every OA location becomes a fallback URL. Unpaywall runs
   for any DOI (including DOIs in `doc.url`), not just `identifier`.
8. Retry HTTP 500 in `get_with_retry`, and make retry sleeps cancel-aware.

---

## Phase 3 — Download engine speed (safe)

| Change | Why |
|---|---|
| Enable reqwest `http2`, `brotli`, `zstd` | HTTP/2 multiplexing to arXiv, OpenAlex, Zenodo, EPMC. Today it's HTTP/1.1 + gzip only. |
| Per-host AIMD limiter + `Retry-After` | Faster on healthy hosts, fewer 429 spirals. The current 2 s/4 s retry ignores `Retry-After`. |
| Fallback URL chain per doc | Try the merged alternates, then the OA locations, then Unpaywall before failing. |
| Reuse already-read bytes | A landing page that sniffs as PDF is currently **downloaded twice** (`downloader.rs:565-570`). |
| Landing-page resolver upgrades | Source-specific rewrites (EPMC → PMC render PDF, Zenodo files API, IA direct file, OSF, bioRxiv/medRxiv `.full.pdf`). Try up to 2 hops. |
| DNS cache shared by the SSRF check and the resolver | Avoids two lookups per connection. |
| 256 KB `BufWriter`; drop per-file `fsync` (keep atomic rename) | Fewer `spawn_blocking` hops per chunk. |
| One SQLite writer task with batched transactions | Replaces a new connection per document (`orchestrator.rs:896`). |
| Content-hash index (`blobs` table, sha256 → path) | In-run dedup of identical PDFs from different URLs, plus instant cross-run reuse. Replaces the O(docs × libraries) `read_dir` scan (`downloader.rs:771`). |
| Cancel-aware `send()` and body reads (`tokio::select!`) | Stop is instant instead of waiting up to 60 s per request. |
| Count panicked tasks as failed | Today done + failed can be less than total. |

---

## Phase 4 — Extraction and AI output

### Extraction (`engine/extract.rs` is currently raw `pdf-extract` with no cleanup)
- **PDF**: switch to **PDFium** (`pdfium-render`, MIT/Apache; PDFium is BSD) with
  the prebuilt per-platform library bundled as a Tauri resource. Keep `pdf-extract`
  as a fallback. We get per-page text, reading order, and font-size cues for
  headings. *Spike first: confirm bundling on all three OSes plus Flatpak, and the
  size increase.*
- **Cleaning pipeline** (pure Rust, unit-tested with fixture PDFs): NFKC plus
  ligature fixes, de-hyphenation across line breaks, removing repeated
  headers/footers and page numbers (detected across pages), merging two-column
  text, collapsing whitespace, a detected heading hierarchy, and splitting out the
  References section.
- **HTML**: readability-style main-content extraction (e.g. `dom_smoothie`), then
  convert to Markdown. Replaces the regex stripper.
- **EPUB**: read the OPF **spine order** (currently zip order), skip nav/TOC, and
  take metadata from the OPF.
- **TXT**: encoding detection (`chardetng` + `encoding_rs`).
- **Scanned PDFs**: detect them and label "scanned — no text layer" clearly.
  OCR is deferred and out of scope for 0.3.
- Store page count, language (`whatlang`), word count and a token estimate per doc.

### Output for AI
- **Per-doc Markdown** (`_text/<slug>.md`) with YAML front matter: title,
  authors, year, DOI, URL, source, license, retrieved date and pages. It
  replaces the headerless `.txt`.
- **Chunks**: about 800-token, heading-aware chunks with stable IDs and page
  ranges, stored in SQLite **FTS5** (already compiled into bundled SQLite).
  This powers in-app full-text search too.
- **AI export pack** (replaces "zip the folder", which currently includes
  `library.db` and the WAL files):
  - `README.md` index (an `llms.txt`-style table of contents with one line per document)
  - `documents/*.md`
  - `chunks.jsonl`
  - `references.bib`
  - an optional single `corpus.md` trimmed to a chosen context budget (e.g.
    100k / 200k / 1M tokens, ranked by relevance)
  - originals, optional
- **"Copy for AI"** in Library: copies a context-budgeted bundle for the current
  library or the current selection to the clipboard.
- **MCP server (recommended)**: an optional "Connect to Claude / other AI apps"
  switch exposes the libraries over the Model Context Protocol (stdio sidecar).
  Tools: `search_library` (FTS5), `get_document`, `get_chunks`,
  `list_libraries`. Any MCP client can then read the corpus directly. This is
  the strongest replacement for on-device AI: the user's own model does the
  reasoning.
- **DB migrations**: add `PRAGMA user_version` migrations. New columns are doi,
  scores, sub-queries, all sources, sha256, pages, lang, tokens and
  retrieved_at, plus the `chunks` FTS table. Today's schema is
  `CREATE TABLE IF NOT EXISTS` only, so this is required before any of the above.

---

## Phase 5 — App redesign (design-ui skill)

**Brief.** The app is a *repeated task* tool, not a brand encounter, so it gets
fast orientation, stable controls and clear state changes.

**Design contract**
- **Purpose:** type a question and get a trustworthy, readable library, with as
  few decisions as possible.
- **Art direction:** *a quiet reading room / card catalog.* Warm paper or deep ink
  fields, hairline rules, catalog numerals (№ 014), and condensed index labels.
  Each document is a catalog slip that gets "filed" as it lands.
- **Reference roles:**
  - **Raycast**: the focused command surface and a clear selected-item inspector.
  - **Locomotive**: editorial hierarchy, ruled lists and quiet navigation.
  - **Remy**: one repeated framing language (ticks and rules marking the active
    item), plus list/detail continuity.
  - **Original**: the catalog-slip motif and the filing motion.
- **Typography:** Instrument Serif (page titles, library names, empty states only)
  with Archivo for everything else. Width stops at 80 (labels), 100 (UI and
  reading) and 112 (rare emphasis), with tabular numerals for all counts. Both
  are **bundled as WOFF2**. The app currently declares Inter and JetBrains Mono
  but never loads them (it falls back to system fonts), and the CSP blocks CDNs
  anyway. Mono is used only for file paths.
- **Surface system:** **Light / Dark / System** (replacing Paper/Slate/Midnight ×
  9 accents × density) and one accent (the logo's blue-green). Five type sizes
  instead of 16. Real shadow and radius tokens. All inline `style={{}}` moves
  into CSS.

**Information architecture**
- **Discover**:
  - Idle shows a big serif prompt, one query field, and a single compact source
    line ("9 sources · edit").
  - Running shows one run view with three plain stages ("Searching → Downloading
    → Reading") instead of eight jargon chips, a live "filed" stream, and a
    single progress figure.
  - Telemetry (MB/s, lanes, sub-queries) collapses into a "Details" drawer.
  - The banner stack becomes one inline status line.
- **Library**: list + inspector (Raycast-style).
  - Searchable across libraries using FTS5.
  - Sortable by relevance, year or source.
  - The inspector shows metadata, abstract, extracted text preview, open,
    reveal, copy-for-AI and remove.
  - Keyboard support: ↑↓, ↵ opens, ⌘F searches, ⌘C copies for AI.
- **Settings**: Appearance, Library folder, Connect to AI (MCP), Data (erase), and About (with the version shown).
- **First run**: an empty-state with three example queries. WelcomeDialog was
  only model onboarding and is deleted.

**Cleanup**
- Delete the ~125-line legacy skeuomorphic shim, the dead CSS, and `ProgressBar.tsx`.
- Fix the `/vite.svg` favicon 404 and the mismatched `theme-color`.
- Accessibility:
  - focus rings on doc rows, summaries and banner buttons
  - nothing under 11px
  - real headings
  - a single live region (not two)
  - keyboard-reachable library items
  - accessible swatch names

**Icons:** a single lucide set for utility glyphs (already a dependency), with
consistent stroke and size. Refresh the app icon per the Parakeet discipline: one
strong silhouette that still reads at 16px. Regenerate with `pnpm tauri icon`.

**Process:** build Discover at full fidelity first, including the 960 px minimum
window, then propagate. Screenshot every state (idle, running, done, empty, error)
in both themes, and review against the contract with `review.md`.

---

## Phase 6 — Site redesign (`site/`)

The site *is* a brand encounter.
- **Direction:** Editorial studio (Locomotive's type hierarchy and ruled lists)
  supporting a precision-software product demonstration (Raycast/Forge: show the
  real task and its result).
- **Hero:** serif headline plus a **real app screenshot or a short looping capture**
  from Phase 5, replacing the 370-line fake HTML mini-window.
- **Sequence:** recognition → proof (real screenshots: Discover running, Library
  inspector, an export opened in an AI chat) → how it works → sources → download.
- Instrument Serif + Archivo, self-hosted (dropping the Google Fonts CDN), using
  the same tokens as the app so the two finally match.
- **Fixes:**
  - a mobile nav (links are `display:none` below 1000 px with no menu)
  - focus styles and a skip link
  - accent button contrast (3.5:1 today, failing AA)
  - a 1200×630 PNG `og:image` (SVG doesn't unfurl)
  - apple-touch-icon and JSON-LD
  - an OS-detecting download button
  - copy rewritten for "no setup, no AI models, works with your AI via MCP"

---

## Phase 7 — Docs and release
- Update README, CLAUDE.md, the release notes heredoc and the site together. AGENTS.md is a near-duplicate of CLAUDE.md, so collapse it into a pointer.
- Bump to **0.3.0** in all three version files.

---

## Other fixes found (folded into the phases above)
- `start_run` doesn't clamp inputs, and the UI allows `maxTotal` = 10000 while
  the store caps the list at 8000.
- "Per-source cap & weights in Settings" hint in `SourcePanel.tsx:154` is false.
- Sub-query telemetry has a vestigial empty count span, and source labels are
  truncated to 4 chars ("Sema", "Euro").
- Library cards are mouse-only, and the detail title isn't a heading.
- The export zip includes `library.db-wal`/`-shm`.
- WKWebView only Tabs to buttons when macOS keyboard navigation is on. Test this
  and add explicit `tabindex` where it matters.

---

## Decisions for Adam
1. **MCP server**: build it in 0.3? *(Recommended: yes. It's the best "no local model" AI story.)*
2. **Bundled static embedding (~8–30 MB)**: only if the benchmark proves a gain? *(Recommended: yes, gated on numbers.)*
3. **PDFium**: bundle the native lib (about +5–8 MB per platform) for much better PDF text? *(Recommended: yes, after a spike.)*
4. **Themes**: cut to Light/Dark/System + one accent? *(Recommended: yes.)*
5. **Linux floor**: move builds to ubuntu-22.04 once ONNX is gone? *(Recommended: yes.)*

## Suggested order and PRs
Phase 0 → 1 → (2 + 3 together) → 4 → 5 → 6 → 7, one PR per phase. Phase 5 design
work can start alongside 2–3 since it only depends on Phase 1's simplified `RunRequest`.
