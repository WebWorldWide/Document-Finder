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
  extract?: boolean;
  use_citation_graph?: boolean;
  use_semantic_rerank?: boolean;
  use_llm_expansion?: boolean;
  use_llm_filter?: boolean;
  llm_model_id?: string | null;
  source_options?: Record<string, { instance_url?: string }>;
}

export type ModelKind = "embedding" | "llm";

export interface ModelInfo {
  id: string;
  kind: ModelKind;
  display_name: string;
  description: string;
  is_default: boolean;
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

export const api = {
  defaultLibraryDir: () => invoke<{ library_root: string }>("default_library_dir"),
  startRun: (req: RunRequest) => invoke<void>("start_run", { req }),
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
  downloadModel: (modelId: string) => invoke<void>("download_model", { modelId }),
  cancelModelDownload: (modelId: string) => invoke<void>("cancel_model_download", { modelId }),
  deleteModel: (modelId: string) => invoke<void>("delete_model", { modelId }),
  deleteLibrary: (path: string) => invoke<void>("delete_library", { path }),
  resetAiState: () => invoke<void>("reset_ai_state"),
};
