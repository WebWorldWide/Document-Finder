import { invoke } from "@tauri-apps/api/core";
import type { SourceId } from "./utils";

export interface Document {
  title: string;
  url: string;
  source: string;
  authors: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
}

export interface RunRequest {
  query: string;
  sources: SourceId[];
  out_dir: string;
  per_source?: number;
  max_total?: number;
  concurrency?: number;
  extract?: boolean;
  source_options?: Record<string, { instance_url?: string }>;
}

export interface LibraryInfo {
  name: string;
  path: string;
  query: string;
  n_docs: number;
  size_bytes: number;
}

export interface ExportResult {
  dest: string;
  files: number;
  size_bytes: number;
}

export interface LogInfo {
  path: string;
  exists: boolean;
  size_bytes: number;
}

export const api = {
  defaultLibraryDir: () =>
    invoke<{ library_root: string }>("default_library_dir"),
  startRun: (req: RunRequest) => invoke<void>("start_run", { req }),
  cancelRun: () => invoke<void>("cancel_run"),
  listLibraries: (root: string) =>
    invoke<LibraryInfo[]>("list_libraries", { root }),
  openLibrary: (path: string) => invoke<LibraryInfo>("open_library", { path }),
  exportLibraryZip: (
    folder: string,
    dest: string,
    opts: { include_text?: boolean; include_originals?: boolean } = {},
  ) =>
    invoke<ExportResult>("export_library_zip", {
      args: {
        folder,
        dest,
        include_text: opts.include_text ?? true,
        include_originals: opts.include_originals ?? true,
      },
    }),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  runLogInfo: () => invoke<LogInfo>("run_log_info"),
  runLogTail: (max?: number) =>
    invoke<unknown[]>("run_log_tail", max != null ? { max } : {}),
  setupSearXNG: () => invoke<string>("setup_searxng"),
};
