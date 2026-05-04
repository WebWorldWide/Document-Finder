import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Document } from "./tauri";

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
export interface SourceErrorPayload {
  source: string;
  error: string;
}
export interface FoundPayload {
  title: string;
  source: string;
  url: string;
  total: number;
}
export interface FoundTotalPayload {
  count: number;
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
export interface DownloadDonePayload extends Document {
  local_path: string;
  absolute_path: string;
  text_path?: string;
  done: number;
  failed: number;
  total: number;
}
export interface DownloadFailedPayload extends Document {
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
export interface FilteredPayload {
  source: string;
  count: number;
}
export interface ErrorPayload {
  message: string;
}

export type DfEvent =
  | { type: "keywords"; payload: KeywordsPayload }
  | { type: "subquery_start"; payload: SubQueryStartPayload }
  | { type: "source_start"; payload: SourceStartPayload }
  | { type: "source_done"; payload: SourceDonePayload }
  | { type: "source_error"; payload: SourceErrorPayload }
  | { type: "found"; payload: FoundPayload }
  | { type: "found_total"; payload: FoundTotalPayload }
  | { type: "download_started"; payload: DownloadStartedPayload }
  | { type: "download_progress"; payload: DownloadProgressPayload }
  | { type: "download_done"; payload: DownloadDonePayload }
  | { type: "download_failed"; payload: DownloadFailedPayload }
  | { type: "cancelled"; payload: CompletePayload }
  | { type: "complete"; payload: CompletePayload }
  | { type: "filtered"; payload: FilteredPayload }
  | { type: "error"; payload: ErrorPayload };

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
  "filtered",
  "error",
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
