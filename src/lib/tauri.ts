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
  use_citation_graph?: boolean;
  use_semantic_rerank?: boolean;
  use_llm_expansion?: boolean;
  use_llm_filter?: boolean;
  llm_model_id?: string | null;
}

export interface RetryRequest {
  /** Existing library folder (must be inside the confined root). */
  folder: string;
  /** Original query, recorded against the retry's run row. */
  query: string;
  /** The documents to re-download (the failed ones from the previous run). */
  docs: Document[];
  concurrency?: number;
  extract?: boolean;
}

export type ModelKind = "embedding" | "llm";

export interface ModelInfo {
  id: string;
  kind: ModelKind;
  display_name: string;
  description: string;
  is_default: boolean;
  license: string;
  approx_bytes: number;
  on_disk_bytes: number;
  status:
    | { kind: "not_downloaded" }
    | { kind: "downloading"; downloaded: number; total: number }
    | { kind: "verifying" }
    | { kind: "ready" }
    | { kind: "failed"; msg: string }
    | { kind: "cancelled" };
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

export interface PurgeReport {
  removed: string[];
  failed: string[];
}

export const api = {
  defaultLibraryDir: () => invoke<{ library_root: string }>("default_library_dir"),
  /** Persist + validate the library root so open/export/delete confine to it. */
  setLibraryRoot: (path: string) => invoke<string>("set_library_root", { path }),
  startRun: (req: RunRequest) => invoke<void>("start_run", { req }),
  /** Re-download a fixed set of documents into an existing library folder. */
  retryRun: (req: RetryRequest) => invoke<void>("retry_run", { req }),
  cancelRun: () => invoke<void>("cancel_run"),
  listLibraries: (root: string) => invoke<LibraryInfo[]>("list_libraries", { root }),
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
  runLogTail: (max?: number) => invoke<unknown[]>("run_log_tail", max != null ? { max } : {}),
  listModels: () => invoke<ModelInfo[]>("list_models"),
  isEmbeddingLoaded: () => invoke<boolean>("is_embedding_loaded"),
  embeddingDownloaded: () => invoke<boolean>("embedding_downloaded"),
  warmEmbedding: () => invoke<void>("warm_embedding"),
  downloadModel: (modelId: string) => invoke<void>("download_model", { modelId }),
  cancelModelDownload: (modelId: string) => invoke<void>("cancel_model_download", { modelId }),
  deleteModel: (modelId: string) => invoke<void>("delete_model", { modelId }),
  deleteLibrary: (path: string) => invoke<void>("delete_library", { path }),
  /** Clean uninstall: erase app data (models, caches, logs); optionally the library too. */
  purgeAllData: (includeLibrary: boolean) =>
    invoke<PurgeReport>("purge_all_data", { includeLibrary }),
  resetAiState: () => invoke<void>("reset_ai_state"),
};
