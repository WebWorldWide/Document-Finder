import { invoke } from "@tauri-apps/api/core";
import type { SourceId } from "./utils";

/** A discovered document (mirror of `sources::Document`). */
export interface Document {
  title: string;
  url: string;
  source: string;
  authors: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
  doi?: string;
  alt_urls?: string[];
  cited_by?: number;
  venue?: string;
  license?: string;
}

/** What to search. How much and how fast is decided by the backend. */
export interface RunRequest {
  query: string;
  sources: SourceId[];
  out_dir: string;
}

export interface RetryRequest {
  folder: string;
  query: string;
  docs: Document[];
}

export interface LibraryInfo {
  name: string;
  path: string;
  query: string;
  n_docs: number;
  size_bytes: number;
}

/** A saved document with full metadata (mirror of `library::DocInfo`). */
export interface LibraryDoc {
  id: number;
  title: string;
  url: string;
  source: string;
  sources: string[];
  authors: string[];
  year?: string | null;
  abstract?: string | null;
  doi?: string | null;
  venue?: string | null;
  license?: string | null;
  cited_by?: number | null;
  score?: number | null;
  /** Absolute path of the original file. */
  path: string;
  /** Absolute path of the extracted Markdown. */
  md_path?: string | null;
  size_bytes?: number | null;
  extract_error?: string | null;
  pages?: number | null;
  lang?: string | null;
  words?: number | null;
  tokens?: number | null;
  retrieved_at?: string | null;
}

export interface SearchHit {
  library: string;
  library_query: string;
  doc_id: number;
  chunk_id: number;
  title: string;
  heading?: string | null;
  page_start?: number | null;
  page_end?: number | null;
  /** Passage with `[` `]` around matched terms. */
  snippet: string;
  rank: number;
}

export interface ExportResult {
  dest: string;
  files: number;
  size_bytes: number;
}

export interface McpInfo {
  command: string;
  args: string[];
  config_json: string;
  claude_code_command: string;
  claude_desktop_config?: string | null;
  claude_desktop_installed: boolean;
}

export interface LogInfo {
  path: string;
  exists: boolean;
  size_bytes: number;
}

export interface PurgeReport {
  removed: string[];
  failed: string[];
}

export const api = {
  defaultLibraryDir: () => invoke<{ library_root: string }>("default_library_dir"),
  setLibraryRoot: (path: string) => invoke<string>("set_library_root", { path }),
  startRun: (req: RunRequest) => invoke<void>("start_run", { req }),
  retryRun: (req: RetryRequest) => invoke<void>("retry_run", { req }),
  cancelRun: () => invoke<void>("cancel_run"),
  listLibraries: (root: string) => invoke<LibraryInfo[]>("list_libraries", { root }),
  openLibrary: (path: string) => invoke<LibraryInfo>("open_library", { path }),
  listLibraryDocs: (path: string) => invoke<LibraryDoc[]>("list_library_docs", { path }),
  searchLibrary: (query: string, path?: string | null, limit?: number) =>
    invoke<SearchHit[]>("search_library", { query, path: path ?? null, limit: limit ?? null }),
  readDocMarkdown: (path: string) => invoke<string>("read_doc_markdown", { path }),
  exportLibraryZip: (
    folder: string,
    dest: string,
    opts: { include_originals?: boolean; budget_tokens?: number } = {},
  ) =>
    invoke<ExportResult>("export_library_zip", {
      args: {
        folder,
        dest,
        include_originals: opts.include_originals ?? false,
        budget_tokens: opts.budget_tokens ?? null,
      },
    }),
  exportLibraryMarkdown: (folder: string, dest: string, ids?: number[] | null) =>
    invoke<ExportResult>("export_library_markdown", { folder, dest, ids: ids ?? null }),
  copyForAi: (folder: string, ids?: number[] | null, budgetTokens?: number) =>
    invoke<string>("copy_for_ai", {
      folder,
      ids: ids ?? null,
      budgetTokens: budgetTokens ?? null,
    }),
  mcpInfo: () => invoke<McpInfo>("mcp_info"),
  installMcpClaudeDesktop: () => invoke<string>("install_mcp_claude_desktop"),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  runLogInfo: () => invoke<LogInfo>("run_log_info"),
  deleteLibrary: (path: string) => invoke<void>("delete_library", { path }),
  purgeAllData: (includeLibrary: boolean) =>
    invoke<PurgeReport>("purge_all_data", { includeLibrary }),
  restartApp: () => invoke<void>("restart_app"),
  /** Linux only (always false elsewhere): WebKitGTK safe rendering for the next launch. */
  getSafeRenderMode: () => invoke<boolean>("get_safe_render_mode"),
  setSafeRenderMode: (enabled: boolean) => invoke<void>("set_safe_render_mode", { enabled }),
};
