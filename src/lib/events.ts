import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface FoundPayload {
  title: string;
  source: string;
  url: string;
  total: number;
}

export interface FoundTotalPayload {
  count: number;
}

export interface KeywordsPayload {
  query: string;
  sub_queries: string[];
}

export interface SubQueryStartPayload {
  sub_query: string;
  keywords: string[];
}

export interface SourceStartPayload {
  source: string;
  sub_query: string;
}

export interface SourceDonePayload {
  source: string;
  count: number;
}

export type SourceErrorKind =
  | "rate_limit"
  | "forbidden"
  | "server_error"
  | "timeout"
  | "parse_error"
  | "other";

export interface SourceErrorPayload {
  source: string;
  error: string;
  kind: SourceErrorKind;
}

export interface DownloadStartedPayload {
  url: string;
  title: string;
  source: string;
}

export interface DownloadProgressPayload {
  url: string;
  title: string;
  downloaded: number;
  total: number;
}

// Rust uses #[serde(flatten)] on doc: Document — fields appear at top level
export interface DownloadDonePayload {
  url: string;
  title: string;
  source: string;
  authors: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
  local_path: string;
  absolute_path: string;
  text_path?: string;
  /** Authoritative on-disk size of the saved file (bytes). */
  bytes: number;
  /** True when reused from a previous run — excluded from network throughput. */
  cached: boolean;
  done: number;
  failed: number;
  total: number;
}

export interface DownloadFailedPayload {
  url: string;
  title: string;
  source: string;
  authors: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
  error: string;
  done: number;
  failed: number;
  total: number;
}

export interface CompletePayload {
  done: number;
  failed: number;
  total: number;
  folder: string;
  manifest: string;
}

export interface ErrorPayload {
  message: string;
}

export interface CandidatePayload {
  title: string;
  url: string;
  source: string; // first-seen source
  authors: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
  sources: string[]; // all sources that returned this candidate
  tfidf: number;
  rrf: number;
  authority: number;
  score: number;
  status: "kept" | "rejected" | "borderline";
  reject_reason: string | null;
  final_rank: number | null;
}

export interface RankingDonePayload {
  total_candidates: number;
  kept: number;
  rejected: number;
}

export interface ModelProgressPayload {
  model_id: string;
  downloaded: number;
  total: number;
  bytes_per_sec: number;
}

export interface ModelStatusPayload {
  model_id: string;
  status:
    | "downloading"
    | "verifying"
    | "ready"
    | "failed"
    | "cancelled"
    | "embedding"
    | "embedding_failed"
    | "llm_warming"
    | "llm_expanding"
    | "llm_filtering";
  detail: string | null;
}

export type PipelineStage =
  | "discovery"
  | "rank"
  | "semantic_rerank"
  | "llm_expand"
  | "llm_filter"
  | "citation_enrich"
  | "download"
  | "extract";

export type PipelineState = "started" | "progress" | "done" | "skipped";

export interface PipelineStagePayload {
  stage: PipelineStage;
  state: PipelineState;
  count?: number;
  total?: number;
  message?: string;
}

export type DfEvent =
  | { type: "found"; payload: FoundPayload }
  | { type: "found_total"; payload: FoundTotalPayload }
  | { type: "keywords"; payload: KeywordsPayload }
  | { type: "subquery_start"; payload: SubQueryStartPayload }
  | { type: "source_start"; payload: SourceStartPayload }
  | { type: "source_done"; payload: SourceDonePayload }
  | { type: "source_error"; payload: SourceErrorPayload }
  | { type: "download_started"; payload: DownloadStartedPayload }
  | { type: "download_progress"; payload: DownloadProgressPayload }
  | { type: "download_done"; payload: DownloadDonePayload }
  | { type: "download_failed"; payload: DownloadFailedPayload }
  | { type: "complete"; payload: CompletePayload }
  | { type: "cancelled"; payload: CompletePayload }
  | { type: "error"; payload: ErrorPayload }
  | { type: "candidate"; payload: CandidatePayload }
  | { type: "ranking_done"; payload: RankingDonePayload }
  | { type: "model_progress"; payload: ModelProgressPayload }
  | { type: "model_status"; payload: ModelStatusPayload };

const EVENTS = [
  "keywords",
  "subquery_start",
  "source_start",
  "source_done",
  "source_error",
  "found",
  "found_total",
  "download_started",
  "download_progress",
  "download_done",
  "download_failed",
  "cancelled",
  "complete",
  "error",
  "candidate",
  "ranking_done",
  "model_progress",
  "model_status",
] as const;

export async function listenAll(handler: (e: DfEvent) => void): Promise<UnlistenFn> {
  const unsubs: UnlistenFn[] = [];
  for (const name of EVENTS) {
    const u = await listen(`df:${name}`, (ev) =>
      handler({ type: name, payload: ev.payload as never }),
    );
    unsubs.push(u);
  }
  return () => unsubs.forEach((u) => u());
}
