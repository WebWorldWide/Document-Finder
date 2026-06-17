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
  /** Set when the file saved but its library index row failed to write — the
   *  file is on disk but won't appear in the Library view. */
  indexError?: string;
  /** Set when the file saved but no usable text could be extracted. */
  extractError?: string;
  /** Document metadata retained on FAILED items so "Retry failed" can rebuild
   *  the Document and re-attempt the download without re-running discovery. */
  authors?: string[];
  year?: string;
  abstract?: string;
  identifier?: string;
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
  /// Accumulated LIVE from `found` events (the authoritative incremental
  /// counter), so the Sources panel climbs in real time during discovery
  /// instead of jumping only when a source's task completes.
  hits: number;
  /// Number of still-running (sub_query, source) tasks for this source. A
  /// source flips to "done" only when this reaches 0, so the status dot
  /// doesn't flicker between querying/done while sibling sub-query tasks for
  /// the same source are still streaming.
  active: number;
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

/// Normalize an event's source id to the key used by the Sources panel.
/// The meta-search aggregator rewrites nested engine ids to
/// "meta_search/<engine>" (see meta_search.rs), but source_start/source_done
/// report the bare aggregator id ("meta_search"). Collapsing the prefix here
/// makes live `found` counts land on the same row as the start/done events.
function baseSourceId(source: string): string {
  return source.startsWith("meta_search/") ? "meta_search" : source;
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
          s.sourceStats[ev.payload.source] = {
            status: "querying",
            hits: cur?.hits ?? 0,
            active: (cur?.active ?? 0) + 1,
          };
        }),
      );
      addLog("info", `   querying ${ev.payload.source}`);
      break;

    case "source_done":
      setState(
        produce((s) => {
          const cur = s.sourceStats[ev.payload.source];
          // One in-flight (sub_query, source) task just finished. Hits are
          // accumulated live from `found`, so do NOT re-add `count` here — that
          // would double-count. Only flip to "done" once the LAST task for this
          // source finishes (active === 0). A source that only errored and found
          // nothing stays "error" rather than masquerading as a clean "done".
          const active = Math.max(0, (cur?.active ?? 1) - 1);
          const hits = cur?.hits ?? 0;
          const status =
            active > 0
              ? cur?.status === "error"
                ? "error"
                : "querying"
              : cur?.status === "error" && hits === 0
                ? "error"
                : "done";
          s.sourceStats[ev.payload.source] = { status, hits, active };
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
          s.sourceStats[source] = {
            status: "error",
            hits: st?.hits ?? 0,
            active: st?.active ?? 0,
          };
        }),
      );
      addLog("warn", `   ${ev.payload.source}: ${ev.payload.error}`);
      break;

    case "found":
      setState(
        produce((s) => {
          // `total` is the cumulative unique (deduped) discovery count.
          s.found = ev.payload.total;
          // Increment the per-source live counter so the Sources panel + the
          // "By source" bar graph climb in real time as documents stream in,
          // instead of only updating when the source's task completes. Collapse
          // the meta_search/<engine> prefix so hits land on the aggregator row.
          const base = baseSourceId(ev.payload.source);
          const cur = s.sourceStats[base];
          s.sourceStats[base] = {
            status: cur?.status === "error" ? "error" : "querying",
            hits: (cur?.hits ?? 0) + 1,
            active: cur?.active ?? 0,
          };
        }),
      );
      break;

    case "found_total":
      setState("total", ev.payload.count);
      // Discovery is over (both waves drained or hit their deadline). Resolve any
      // source still showing "querying": when a slow source's task is aborted on
      // the wave deadline it never emits source_done, which would otherwise leave
      // its card spinning through the rank/download/extract phases. (Wider LLM
      // fan-out makes more tasks hit the deadline, so this is now common.)
      setState(
        produce((s) => {
          for (const id of Object.keys(s.sourceStats)) {
            const st = s.sourceStats[id];
            if (!st) continue;
            if (st.status === "querying") {
              s.sourceStats[id] = { status: "done", hits: st.hits, active: 0 };
            } else if (st.active !== 0) {
              // Zero any leftover in-flight count (e.g. an aborted task whose
              // source_done never arrived) without disturbing an error row.
              s.sourceStats[id] = { ...st, active: 0 };
            }
          }
        }),
      );
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
        indexError: ev.payload.index_error,
        extractError: ev.payload.extract_error,
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
        // Retain full doc metadata so "Retry failed" can rebuild the Document.
        authors: ev.payload.authors,
        year: ev.payload.year,
        abstract: ev.payload.abstract,
        identifier: ev.payload.identifier,
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

  // Connectivity pre-check: every source needs the network, so a definitively
  // offline machine would otherwise produce a confusing cascade of per-source
  // errors. navigator.onLine is only trustworthy when false (the OS reports no
  // link), so we block solely on that and let real searches proceed otherwise.
  if (typeof navigator !== "undefined" && navigator.onLine === false) {
    reset(query.trim());
    setState("fatalError", "You appear to be offline. Connect to the internet and try again.");
    addLog("error", "Offline — searches need an internet connection.");
    return;
  }

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

/// Re-attempt the downloads that failed in the just-finished run, WITHOUT
/// re-running discovery/ranking. Modeled as a fresh, scoped run (reset + the
/// normal EV_DOWNLOAD_*/EV_COMPLETE flow) over just the failed docs, so the run
/// card tracks it with correct counts and no cumulative-counter gymnastics. The
/// recovered files land in the same library folder.
async function retryFailed() {
  if (state.running || !state.folder) return;
  const failed = state.completed.filter((c) => c.status === "failed");
  if (failed.length === 0) return;

  const folder = state.folder;
  const query = state.query;
  const docs = failed.map((c) => ({
    title: c.title,
    url: c.url,
    source: c.source,
    authors: c.authors ?? [],
    year: c.year,
    abstract: c.abstract,
    identifier: c.identifier,
  }));

  reset(query);
  // Seed `found` with the retry scope so the run card reads "N found · M saved"
  // during the retry (there's no discovery phase to populate it).
  setState({ running: true, folder, found: docs.length });
  pipelineStore.reset();
  void pipelineStore.ensureSubscribed();

  try {
    await api.retryRun({
      folder,
      query,
      docs,
      concurrency: settings.concurrency,
      extract: true,
    });
  } catch (e) {
    // A retry_run rejection (concurrent-run guard, folder outside root) means
    // the retry never started — surface it without the AI-evicting `error` path.
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
  retryFailed,
  exportZip,
  /// Reset all live run state. Called when starting a new query; also used by
  /// unit tests to isolate the event-reducer between cases.
  reset(query = "") {
    reset(query);
  },
  clearFatalError() {
    setState("fatalError", null);
  },
};
