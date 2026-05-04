import { create } from "zustand";
import type { DfEvent } from "@/lib/events";

export interface InFlight {
  url: string;
  title: string;
  source: string;
  startedAt: number;
  downloaded: number;
  total: number;
}

export interface CompletedItem {
  url: string;
  title: string;
  source: string;
  status: "done" | "failed";
  error?: string;
  local_path?: string;
  absolute_path?: string;
  text_path?: string;
}

export interface SourceIssue {
  source: string;
  error: string;
  ts: number;
}

export interface RunState {
  running: boolean;
  query: string;
  subQueries: string[];
  found: number;
  done: number;
  failed: number;
  total: number;
  active: number;
  filteredCount: number;
  inFlight: Record<string, InFlight>;
  completed: CompletedItem[];
  sourceIssues: SourceIssue[];
  log: { ts: number; level: "info" | "warn" | "error"; msg: string }[];
  folder: string | null;
  manifest: string | null;
  fatalError: string | null;

  apply(ev: DfEvent): void;
  reset(query: string): void;
  setRunning(running: boolean): void;
}

export const useRunStore = create<RunState>((set) => ({
  running: false,
  query: "",
  subQueries: [],
  found: 0,
  done: 0,
  failed: 0,
  total: 0,
  active: 0,
  filteredCount: 0,
  inFlight: {},
  completed: [],
  sourceIssues: [],
  log: [],
  folder: null,
  manifest: null,
  fatalError: null,

  reset: (query) =>
    set({
      query,
      subQueries: [],
      found: 0,
      done: 0,
      failed: 0,
      total: 0,
      active: 0,
      filteredCount: 0,
      inFlight: {},
      completed: [],
      sourceIssues: [],
      log: [],
      folder: null,
      manifest: null,
      fatalError: null,
    }),
  setRunning: (running) => set({ running }),
  apply: (ev) =>
    set((s) => {
      const log = (level: "info" | "warn" | "error", msg: string) =>
        [...s.log, { ts: Date.now(), level, msg }].slice(-200);

      switch (ev.type) {
        case "keywords":
          return { subQueries: ev.payload.sub_queries };
        case "subquery_start":
          return { log: log("info", `→ ${ev.payload.sub_query}`) };
        case "source_start":
          return { log: log("info", `   query ${ev.payload.source}`) };
        case "source_done":
          return {
            log: log("info", `   ${ev.payload.source}: +${ev.payload.count}`),
          };
        case "source_error":
          return {
            sourceIssues: [
              ...s.sourceIssues,
              {
                source: ev.payload.source,
                error: ev.payload.error,
                ts: Date.now(),
              },
            ].slice(-50),
            log: log(
              "warn",
              `   ${ev.payload.source}: ${ev.payload.error}`,
            ),
          };
        case "found":
          return { found: ev.payload.total };
        case "found_total":
          return {
            total: ev.payload.count,
            log: log(
              "info",
              `Discovery complete — ${ev.payload.count} candidate(s)`,
            ),
          };
        case "download_started": {
          const inFlight = {
            ...s.inFlight,
            [ev.payload.url]: {
              url: ev.payload.url,
              title: ev.payload.title,
              source: ev.payload.source,
              startedAt: Date.now(),
              downloaded: 0,
              total: 0,
            },
          };
          return { inFlight, active: Object.keys(inFlight).length };
        }
        case "download_progress": {
          const cur = s.inFlight[ev.payload.url];
          if (!cur) return {};
          return {
            inFlight: {
              ...s.inFlight,
              [ev.payload.url]: {
                ...cur,
                downloaded: ev.payload.downloaded,
                total: ev.payload.total,
              },
            },
          };
        }
        case "download_done": {
          const { [ev.payload.url]: _, ...rest } = s.inFlight;
          const item: CompletedItem = {
            url: ev.payload.url,
            title: ev.payload.title,
            source: ev.payload.source,
            status: "done",
            local_path: ev.payload.local_path,
            absolute_path: ev.payload.absolute_path,
            text_path: ev.payload.text_path,
          };
          return {
            inFlight: rest,
            active: Object.keys(rest).length,
            done: ev.payload.done,
            failed: ev.payload.failed,
            total: ev.payload.total,
            completed: [...s.completed, item].slice(-500),
          };
        }
        case "download_failed": {
          const { [ev.payload.url]: _, ...rest } = s.inFlight;
          const item: CompletedItem = {
            url: ev.payload.url,
            title: ev.payload.title,
            source: ev.payload.source,
            status: "failed",
            error: ev.payload.error,
          };
          return {
            inFlight: rest,
            active: Object.keys(rest).length,
            done: ev.payload.done,
            failed: ev.payload.failed,
            total: ev.payload.total,
            completed: [...s.completed, item].slice(-500),
          };
        }
        case "complete":
        case "cancelled":
          return {
            running: false,
            folder: ev.payload.folder,
            manifest: ev.payload.manifest,
            done: ev.payload.done,
            failed: ev.payload.failed,
            total: ev.payload.total,
            log: log(
              ev.type === "cancelled" ? "warn" : "info",
              ev.type === "cancelled"
                ? `Cancelled. Saved ${ev.payload.done} file(s).`
                : `Done. ${ev.payload.done} saved, ${ev.payload.failed} failed.`,
            ),
          };
        case "filtered":
          return {
            filteredCount: s.filteredCount + ev.payload.count,
            log: log(
              "info",
              `   ${ev.payload.source}: filtered ${ev.payload.count} off-topic`,
            ),
          };
        case "error":
          return {
            running: false,
            fatalError: ev.payload.message,
            log: log("error", `Error: ${ev.payload.message}`),
          };
        default:
          return {};
      }
    }),
}));
