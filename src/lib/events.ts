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
  /** Unique-NEW docs this (sub_query, source) task added to the dedup (merged
   *  cross-source duplicates excluded), not raw results. Per-source hit counts
   *  are accumulated live from `found` events; this is informational. */
  count: number;
}

export type SourceErrorKind =
  "rate_limit" | "forbidden" | "server_error" | "timeout" | "parse_error" | "other";

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
  /** Set when the file saved but its SQLite index row failed to write — the doc
   *  counts as done but won't appear in the Library view. Surfaced as a warning. */
  index_error?: string;
  /** Set when the file saved but no usable text could be extracted (e.g. a
   *  scanned image-only PDF). The file is kept; surfaced as a low-key count. */
  extract_error?: string;
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

/** A started document dropped without counting as a failure. */
export interface DownloadSkippedPayload {
  url: string;
  title: string;
  source: string;
  reason: string;
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
  relevance: number;
  rrf: number;
  authority: number;
  score: number;
  status: "kept" | "rejected";
  reject_reason: string | null;
  final_rank: number | null;
}

/** How many documents the run currently plans to download (grows). */
export interface PlanPayload {
  planned: number;
}

export interface RankingDonePayload {
  total_candidates: number;
  kept: number;
  rejected: number;
}

export type PipelineStage = "discovery" | "rank" | "download" | "extract";

export type PipelineState = "started" | "progress" | "done" | "skipped";

export interface PipelineStagePayload {
  stage: PipelineStage;
  state: PipelineState;
  count?: number;
  total?: number;
  message?: string;
}

/// Per-engine web-search health (df:meta_search_health). Consumed by a dedicated
/// listener in MetaSearchHealthBar.tsx, NOT through listenAll/DfEvent — it's a
/// standalone diagnostic stream, not part of the run reducer. Mirror of
/// events.rs `MetaSearchHealthPayload`.
export type MetaSearchHealthStatus =
  | "ok"
  // "empty" (healthy, zero results), "partial" (slow but returned results), and
  // "throttled" (rate-limited, not broken) are non-failing states the backend
  // emits so they don't trip the circuit breaker.
  | "empty"
  | "partial"
  | "throttled"
  | "timeout"
  | "circuit_open"
  | "error";

export interface MetaSearchHealthPayload {
  backend: string;
  status: MetaSearchHealthStatus;
  result_count: number;
  latency_ms: number;
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
  | { type: "download_skipped"; payload: DownloadSkippedPayload }
  | { type: "complete"; payload: CompletePayload }
  | { type: "cancelled"; payload: CompletePayload }
  | { type: "error"; payload: ErrorPayload }
  | { type: "candidate"; payload: CandidatePayload }
  | { type: "ranking_done"; payload: RankingDonePayload }
  | { type: "plan"; payload: PlanPayload }
  | { type: "pipeline_stage"; payload: PipelineStagePayload }
  | { type: "meta_search_health"; payload: MetaSearchHealthPayload };

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
  "download_skipped",
  "cancelled",
  "complete",
  "error",
  "candidate",
  "ranking_done",
  "plan",
  "pipeline_stage",
  "meta_search_health",
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
