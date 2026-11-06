// Dev-only mock backend: lets the UI run in a normal browser (`pnpm dev`)
// for design review and screenshots. Loaded only when Tauri is absent and
// import.meta.env.DEV is true — never part of a production bundle.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type { LibraryDoc, LibraryInfo, SearchHit } from "@/lib/tauri";

const ROOT = "/Users/demo/Documents/Document Finder/library";
const now = Math.floor(Date.now() / 1000);

const PAPERS: [string, string, string[], string][] = [
  ["Attention Is All You Need", "arxiv", ["Ashish Vaswani", "Noam Shazeer", "Niki Parmar"], "2017"],
  [
    "Sleep-dependent memory consolidation in adolescents",
    "europe_pmc",
    ["R. Kopasz", "K. Hoedlmoser"],
    "2010",
  ],
  [
    "The role of sleep in declarative memory consolidation",
    "openalex",
    ["Jan Born", "Ines Wilhelm"],
    "2012",
  ],
  ["Sleep spindles and memory reprocessing", "semantic_scholar", ["Monika Schönauer"], "2016"],
  ["School start times, sleep, and academic performance", "doaj", ["Kyla Wahlstrom"], "2014"],
  [
    "Adolescent sleep patterns: a systematic review",
    "europe_pmc",
    ["M. Gradisar", "G. Gardner", "H. Dohnt"],
    "2011",
  ],
  ["Memory consolidation during slow-wave sleep", "arxiv", ["L. Marshall"], "2019"],
  ["Sleep deprivation and working memory in teenagers", "zenodo", ["A. Ruiz"], "2021"],
  ["The Interpretation of Dreams", "gutenberg", ["Sigmund Freud"], "1913"],
  ["Sleep, learning and the developing brain", "openalex", ["Matthew Walker"], "2018"],
  ["REM sleep and emotional memory processing", "semantic_scholar", ["E. van der Helm"], "2011"],
  [
    "Screen time before bed and sleep quality in adolescents",
    "meta_search/web",
    ["L. Hale"],
    "2015",
  ],
];

const libs: LibraryInfo[] = [
  {
    name: `effects-of-sleep-on-memory-in-teenagers-${now - 3600}-1`,
    path: `${ROOT}/effects-of-sleep-on-memory-in-teenagers`,
    query: "effects of sleep on memory in teenagers",
    n_docs: 184,
    size_bytes: 412_000_000,
  },
  {
    name: `roman-roads-and-trade-networks-${now - 86400 * 2}-1`,
    path: `${ROOT}/roman-roads-and-trade-networks`,
    query: "Roman roads and trade networks",
    n_docs: 97,
    size_bytes: 268_000_000,
  },
  {
    name: `perovskite-solar-cell-stability-${now - 86400 * 9}-1`,
    path: `${ROOT}/perovskite-solar-cell-stability`,
    query: "perovskite solar cell stability",
    n_docs: 241,
    size_bytes: 702_000_000,
  },
];

function docsFor(lib: string): LibraryDoc[] {
  return PAPERS.map(([title, source, authors, year], i) => ({
    id: i + 1,
    title,
    url: `https://example.org/${i}.pdf`,
    source: source.split("/")[0],
    sources: i % 3 === 0 ? [source.split("/")[0], "openalex"] : [source.split("/")[0]],
    authors,
    year,
    abstract:
      i === 0
        ? "The dominant sequence transduction models are based on complex recurrent or convolutional neural networks. We propose a new simple network architecture, the Transformer, based solely on attention mechanisms."
        : "We examine how sleep architecture supports the consolidation of newly learned information, combining polysomnography with behavioural measures across age groups.",
    doi: i % 2 === 0 ? `10.1000/demo.${i}` : null,
    venue: i % 2 === 0 ? "Journal of Sleep Research" : null,
    license: "cc-by",
    cited_by: 1200 - i * 90,
    score: 10 - i,
    path: `${lib}/${i}.pdf`,
    md_path: i === 8 ? null : `${lib}/_text/${i}.md`,
    size_bytes: 400_000 + i * 120_000,
    extract_error: i === 8 ? "This PDF is a scanned image with no text layer" : null,
    pages: 8 + i,
    lang: "eng",
    words: 6000 + i * 400,
    tokens: 9000 + i * 600,
  }));
}

const MD = `---
title: "Sleep-dependent memory consolidation"
---

# Sleep-dependent memory consolidation

> **Abstract.** We examine how sleep architecture supports the consolidation of newly learned information.

<!-- page 1 -->

## 1 Introduction

Sleep is increasingly recognised as an active state for memory. During slow-wave sleep, hippocampal replay coordinates with thalamocortical spindles, strengthening cortical representations of recent experience.

Adolescence is a period of profound change in sleep timing and architecture. Later circadian phase combined with early school start times produces chronic sleep restriction in a large share of teenagers.

## 2 Methods

Forty-two participants aged 13–17 completed a paired-associate learning task before a night of polysomnographically recorded sleep.

<!-- page 2 -->

### 2.1 Measures

Recall was tested immediately and after 12 hours. Spindle density was computed over central derivations.
`;

function hits(q: string): SearchHit[] {
  return PAPERS.slice(1, 6).map(([title], i) => ({
    library: libs[0].path,
    library_query: libs[0].query,
    doc_id: i + 2,
    chunk_id: i,
    title,
    heading: i % 2 ? "Results" : "Introduction",
    page_start: i + 2,
    page_end: i + 2,
    snippet: `…participants who slept showed stronger [${q}] retention than those kept awake, consistent with an active role of sleep in…`,
    rank: -10 + i,
  }));
}

async function simulateRun(query: string) {
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  await emit("df:keywords", { query, sub_queries: [query, "sleep memory", "teenagers memory"] });
  let total = 0;
  for (const [title, source] of PAPERS) {
    total += 7;
    await emit("df:found", { title, source, url: `https://x/${total}`, total });
    await wait(60);
  }
  await emit("df:plan", { planned: 46 });
  let done = 0;
  let failed = 0;
  for (let i = 0; i < 46; i++) {
    const [title, source, authors, year] = PAPERS[i % PAPERS.length];
    const url = `https://example.org/run/${i}.pdf`;
    await emit("df:download_started", { url, title, source });
    await emit("df:download_progress", { url, title, downloaded: 200_000, total: 900_000 });
    await wait(i < 3 ? 600 : 260);
    if (i % 9 === 4) {
      failed++;
      await emit("df:download_failed", {
        url,
        title,
        source,
        authors,
        year,
        error:
          "This source blocked the download — it may need a subscription or sign-in. (HTTP 403)",
        done,
        failed,
        total: 46,
      });
    } else {
      done++;
      await emit("df:download_done", {
        url,
        title,
        source,
        authors,
        year,
        local_path: `${i}.pdf`,
        absolute_path: `/tmp/${i}.pdf`,
        text_path: i === 8 ? null : `_text/${i}.md`,
        bytes: 500_000 + i * 31_000,
        cached: i === 5,
        extract_error: i === 8 ? "scanned" : null,
        done,
        failed,
        total: 46,
      });
    }
  }
  await emit("df:found_total", { count: total });
  await emit("df:ranking_done", { total_candidates: total, kept: 60, rejected: 24 });
  await emit("df:complete", { done, failed, total: done, folder: libs[0].path, manifest: "" });
}

export function installMockBackend() {
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "default_library_dir":
          return { library_root: ROOT };
        case "set_library_root":
          return a.path;
        case "list_libraries":
          return libs;
        case "list_library_docs":
          return docsFor(String(a.path));
        case "search_library":
          return hits(String(a.query));
        case "read_doc_markdown":
          return MD;
        case "start_run":
          void simulateRun((a.req as { query: string }).query);
          return null;
        case "export_library_markdown":
          return { dest: String(a.dest), files: 12, size_bytes: 1_400_000 };
        case "copy_for_ai":
          return "# corpus\n".repeat(2000);
        case "mcp_info":
          return {
            command: "/Applications/Document Finder.app/Contents/MacOS/document-finder",
            args: ["--mcp"],
            config_json: JSON.stringify(
              {
                mcpServers: {
                  "document-finder": {
                    command: "/Applications/Document Finder.app/Contents/MacOS/document-finder",
                    args: ["--mcp"],
                  },
                },
              },
              null,
              2,
            ),
            claude_code_command:
              'claude mcp add document-finder -- "/Applications/Document Finder.app/Contents/MacOS/document-finder" --mcp',
            claude_desktop_config:
              "~/Library/Application Support/Claude/claude_desktop_config.json",
            claude_desktop_installed: false,
          };
        case "install_mcp_claude_desktop":
          return "Added. Restart Claude Desktop to connect.";
        case "run_log_info":
          return {
            path: "~/Library/Logs/document-finder/runs.jsonl",
            exists: true,
            size_bytes: 182_000,
          };
        case "plugin:app|version":
          return "0.3.0";
        case "get_safe_render_mode":
          return false;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
