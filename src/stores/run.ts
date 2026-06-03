import { createStore, produce } from "solid-js/store";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, type ExportResult } from "@/lib/tauri";
import type { CandidatePayload, DfEvent } from "@/lib/events";
import { settings, qualityToFlags } from "@/stores/settings";
import { pipelineStore } from "@/stores/pipeline";

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
  /** On-disk size of the saved file (bytes); undefined for failures. */
  bytes?: number;
  /** True when the file was reused from a previous run (no network transfer). */
  cached?: boolean;
}

export interface SourceIssue {
  source: string;
  error: string;
  /// One of: rate_limit, forbidden, server_error, timeout, parse_error, other.
  /// Used for dedup — repeated errors of the same (source, kind) combine
  /// into a single row with a count badge instead of stacking.
  kind: string;
  count: number;
  ts: number;
}

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  msg: string;
}

export interface SourceStat {
  /// Live discovery phase for this source in the current run.
  status: "querying" | "done" | "error";
  /// Cumulative hits this source returned across sub-queries this run.
  hits: number;
}

export type Candidate = CandidatePayload;

interface RunState {
  running: boolean;
  query: string;
  subQueries: string[];
  found: number;
  done: number;
  failed: number;
  total: number;
  active: number;
  inFlight: Record<string, InFlight>;
  completed: CompletedItem[];
  sourceIssues: SourceIssue[];
  log: LogEntry[];
  folder: string | null;
  manifest: string | null;
  fatalError: string | null;
  // New for B3: ranked candidates with full scoring + reject reason.
  candidates: Candidate[];
  rankingDone: boolean;
  rankingKept: number;
  rankingRejected: number;
  /// Per-source live status + hits, keyed by source id (for the Sources panel).
  sourceStats: Record<string, SourceStat>;
  /// Cumulative bytes pulled this run — sampled by Discover for throughput.
  bytesDownloaded: number;
}

const [state, setState] = createStore<RunState>({
  running: false,
  query: "",
  subQueries: [],
  found: 0,
  done: 0,
  failed: 0,
  total: 0,
  active: 0,
  inFlight: {},
  completed: [],
  sourceIssues: [],
  log: [],
  folder: null,
  manifest: null,
  fatalError: null,
  candidates: [],
  rankingDone: false,
  rankingKept: 0,
  rankingRejected: 0,
  sourceStats: {},
  bytesDownloaded: 0,
});

function addLog(level: LogEntry["level"], msg: string) {
  setState("log", (prev) => [...prev, { ts: Date.now(), level, msg }].slice(-200));
}

function reset(query: string) {
  setState({
    running: false,
    query,
    subQueries: [],
    found: 0,
    done: 0,
    failed: 0,
    total: 0,
    active: 0,
    inFlight: {},
    completed: [],
    sourceIssues: [],
    log: [],
    folder: null,
    manifest: null,
    fatalError: null,
    candidates: [],
    rankingDone: false,
    rankingKept: 0,
    rankingRejected: 0,
    sourceStats: {},
    bytesDownloaded: 0,
  });
}

function apply(ev: DfEvent) {
  switch (ev.type) {
    case "keywords":
      setState("subQueries", ev.payload.sub_queries);
      break;

    case "subquery_start":
      addLog("info", `→ ${ev.payload.sub_query}`);
      break;

    case "source_start":
      setState(
        produce((s) => {
          const cur = s.sourceStats[ev.payload.source];
          s.sourceStats[ev.payload.source] = { status: "querying", hits: cur?.hits ?? 0 };
        }),
      );
      addLog("info", `   querying ${ev.payload.source}`);
      break;

    case "source_done":
      setState(
        produce((s) => {
          const cur = s.sourceStats[ev.payload.source];
          s.sourceStats[ev.payload.source] = {
            status: "done",
            hits: (cur?.hits ?? 0) + ev.payload.count,
          };
        }),
      );
      addLog("info", `   ${ev.payload.source}: +${ev.payload.count}`);
      break;

    case "source_error":
      // Dedup by (source, kind) — repeats of the same category bump the
      // count on the existing row instead of stacking new ones. The
      // backend already drops parse_error before emitting and dedups
      // within a single task, so the frontend just needs to handle the
      // cross-task / cross-subquery overlap.
      setState(
        produce((s) => {
          const { source, error, kind } = ev.payload;
          const existing = s.sourceIssues.find((i) => i.source === source && i.kind === kind);
          if (existing) {
            existing.count += 1;
            existing.ts = Date.now();
            // Keep the freshest message verbatim — useful when the backend
            // includes a slightly different detail each time.
            existing.error = error;
          } else {
            s.sourceIssues = [
              ...s.sourceIssues,
              { source, error, kind, count: 1, ts: Date.now() },
            ].slice(-50);
          }
          const st = s.sourceStats[source];
          s.sourceStats[source] = { status: "error", hits: st?.hits ?? 0 };
        }),
      );
      addLog("warn", `   ${ev.payload.source}: ${ev.payload.error}`);
      break;

    case "found":
      setState("found", ev.payload.total);
      break;

    case "found_total":
      setState("total", ev.payload.count);
      addLog("info", `Discovery complete — ${ev.payload.count} candidate(s)`);
      break;

    case "download_started":
      setState(
        produce((s) => {
          s.inFlight[ev.payload.url] = {
            url: ev.payload.url,
            title: ev.payload.title,
            source: ev.payload.source,
            startedAt: Date.now(),
            downloaded: 0,
            total: 0,
          };
          s.active = Object.keys(s.inFlight).length;
        }),
      );
      break;

    case "download_progress":
      if (state.inFlight[ev.payload.url]) {
        // Accumulate the positive delta so Discover can sample a throughput
        // sparkline without tracking per-url byte history itself.
        const delta = ev.payload.downloaded - state.inFlight[ev.payload.url].downloaded;
        if (delta > 0) setState("bytesDownloaded", (b) => b + delta);
        setState("inFlight", ev.payload.url, "downloaded", ev.payload.downloaded);
        setState("inFlight", ev.payload.url, "total", ev.payload.total);
      }
      break;

    case "download_done": {
      // Capture the provisional bytes already counted for this url from throttled
      // progress events BEFORE deleting the in-flight entry, then reconcile the
      // cumulative network total to the authoritative on-disk size. Cached files
      // were reused from a prior run (no network transfer), so they contribute 0
      // to throughput. This makes the throughput graph exact at every completion
      // and self-heals any progress-throttle gaps.
      const prev = state.inFlight[ev.payload.url]?.downloaded ?? 0;
      const contribution = ev.payload.cached ? 0 : ev.payload.bytes;
      const item: CompletedItem = {
        url: ev.payload.url,
        title: ev.payload.title,
        source: ev.payload.source,
        status: "done",
        local_path: ev.payload.local_path,
        absolute_path: ev.payload.absolute_path,
        text_path: ev.payload.text_path,
        bytes: ev.payload.bytes,
        cached: ev.payload.cached,
      };
      setState(
        produce((s) => {
          s.bytesDownloaded = Math.max(0, s.bytesDownloaded + contribution - prev);
          delete s.inFlight[ev.payload.url];
          s.active = Object.keys(s.inFlight).length;
          s.done = ev.payload.done;
          s.failed = ev.payload.failed;
          s.total = ev.payload.total;
          s.completed = [...s.completed, item].slice(-500);
        }),
      );
      break;
    }

    case "download_failed": {
      // Roll back any provisional bytes counted for this url: the partial file is
      // deleted on disk, so its streamed bytes must not linger in the throughput
      // total (which would inflate avg MB/s for content that no longer exists).
      const prev = state.inFlight[ev.payload.url]?.downloaded ?? 0;
      const item: CompletedItem = {
        url: ev.payload.url,
        title: ev.payload.title,
        source: ev.payload.source,
        status: "failed",
        error: ev.payload.error,
      };
      setState(
        produce((s) => {
          s.bytesDownloaded = Math.max(0, s.bytesDownloaded - prev);
          delete s.inFlight[ev.payload.url];
          s.active = Object.keys(s.inFlight).length;
          s.done = ev.payload.done;
          s.failed = ev.payload.failed;
          s.total = ev.payload.total;
          s.completed = [...s.completed, item].slice(-500);
        }),
      );
      break;
    }

    case "complete":
    case "cancelled":
      setState({
        running: false,
        inFlight: {},
        active: 0,
        folder: ev.payload.folder,
        manifest: ev.payload.manifest,
        done: ev.payload.done,
        failed: ev.payload.failed,
        total: ev.payload.total,
        // A clean terminal state clears any earlier (e.g. task-panic) error so a
        // stale fatal-error banner doesn't linger over a run that finished.
        fatalError: null,
      });
      addLog(
        ev.type === "cancelled" ? "warn" : "info",
        ev.type === "cancelled"
          ? `Cancelled. Saved ${ev.payload.done} file(s).`
          : `Done. ${ev.payload.done} saved, ${ev.payload.failed} failed.`,
      );
      break;

    case "error":
      setState({ running: false, fatalError: ev.payload.message });
      addLog("error", `Error: ${ev.payload.message}`);
      // Reset AI singletons so the next search can re-initialize them cleanly
      // without requiring an app restart after an inference crash.
      api.resetAiState().catch((e) => console.error("reset_ai_state failed:", e));
      break;

    case "candidate":
      setState(
        produce((s) => {
          // Replace prior entry for the same URL (re-emit case) or append.
          const idx = s.candidates.findIndex((c) => c.url === ev.payload.url);
          if (idx >= 0) {
            s.candidates[idx] = ev.payload;
          } else {
            s.candidates.push(ev.payload);
          }
        }),
      );
      break;

    case "ranking_done":
      setState({
        rankingDone: true,
        rankingKept: ev.payload.kept,
        rankingRejected: ev.payload.rejected,
      });
      addLog(
        "info",
        `Ranked ${ev.payload.total_candidates} candidate(s): ${ev.payload.kept} kept, ${ev.payload.rejected} rejected`,
      );
      break;
  }
}

async function startSearch(query: string) {
  if (!query.trim() || state.running) return;
  if (settings.selectedSources.length === 0) return;

  reset(query.trim());
  // Pipeline strip should clear from the previous run before stage events
  // for the new run start arriving.
  pipelineStore.reset();
  void pipelineStore.ensureSubscribed();
  setState("running", true);

  try {
    const flags = qualityToFlags(settings.quality);
    await api.startRun({
      query: query.trim(),
      sources: settings.selectedSources,
      out_dir: settings.libraryRoot,
      per_source: settings.perSource,
      max_total: settings.maxTotal,
      // Was silently dropped before, so the backend always used its default of
      // 8 and the user's "Parallel downloads" / intensity setting did nothing.
      concurrency: settings.concurrency,
      extract: true,
      use_citation_graph: settings.useCitationGraph,
      ...flags,
      llm_model_id: settings.llmModelId || null,
    });
  } catch (e) {
    // A start_run rejection (the concurrent-run guard, or a library folder
    // outside the allowed root) means the pipeline never started — it is NOT an
    // inference crash, so surface the error WITHOUT routing through the `error`
    // event handler, which would needlessly evict warmed AI models and force a
    // multi-second re-warm on the next search.
    setState({ running: false, fatalError: String(e) });
    addLog("error", `Error: ${String(e)}`);
  }
}

async function exportZip(): Promise<ExportResult | null> {
  if (!state.folder) return null;

  // Split on BOTH separators: state.folder is an OS-native path, so on Windows
  // it is backslash-separated and split("/") would return the whole path as the
  // "slug", producing an invalid pre-filled ZIP name.
  const slug = state.folder.split(/[\\/]/).pop() || "library";
  const dest = await saveDialog({
    defaultPath: `${slug}.zip`,
    filters: [{ name: "ZIP archive", extensions: ["zip"] }],
  });

  if (!dest) return null;

  const result = await api.exportLibraryZip(state.folder, dest);
  await api.revealInFinder(result.dest);
  return result;
}

export const runStore = {
  get state() {
    return state;
  },
  get overallPct() {
    return state.total > 0 ? Math.round(((state.done + state.failed) / state.total) * 100) : 0;
  },
  apply,
  startSearch,
  exportZip,
  clearFatalError() {
    setState("fatalError", null);
  },
};
