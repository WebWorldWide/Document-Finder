import { createStore, produce } from "solid-js/store";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api, type ExportResult } from "@/lib/tauri";
import type { DfEvent } from "@/lib/events";
import { settings } from "@/stores/settings";

export interface InFlight {
  task_id: string;
  url: string;
  title: string;
  source: string;
  startedAt: number;
  downloaded: number;
  total: number;
}

export interface CompletedItem {
  task_id: string;
  url: string;
  title: string;
  source: string;
  status: "done" | "failed";
  error?: string;
  identifier?: string;
  local_path?: string;
  absolute_path?: string;
  text_path?: string;
}

export interface SourceIssue {
  source: string;
  error: string;
  ts: number;
}

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  msg: string;
}

export type SourceStatus = {
  phase: "querying" | "done" | "error";
  doneCount?: number;
};

export interface SubQueryState {
  text: string;
  found: number;
  done: number;
}

interface RunState {
  running: boolean;
  query: string;
  subQueries: SubQueryState[];
  found: number;
  done: number;
  failed: number;
  total: number;
  active: number;
  filteredCount: number;
  inFlight: Record<string, InFlight>;
  completed: CompletedItem[];
  sourceIssues: SourceIssue[];
  sourceStatus: Record<string, SourceStatus>;
  log: LogEntry[];
  folder: string | null;
  manifest: string | null;
  fatalError: string | null;
  /// Last 32 MB/s samples — populated by a 600ms ticker while running.
  speedHist: number[];
  /// Cumulative bytes downloaded across all in-flight + completed items.
  /// Used by the speed ticker to compute MB/s deltas.
  bytesAccum: number;
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
  filteredCount: 0,
  inFlight: {},
  completed: [],
  sourceIssues: [],
  sourceStatus: {},
  log: [],
  folder: null,
  manifest: null,
  fatalError: null,
  speedHist: [],
  bytesAccum: 0,
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
    filteredCount: 0,
    inFlight: {},
    completed: [],
    sourceIssues: [],
    sourceStatus: {},
    log: [],
    folder: null,
    manifest: null,
    fatalError: null,
    speedHist: [],
    bytesAccum: 0,
  });
}

function apply(ev: DfEvent) {
  switch (ev.type) {
    case "keywords":
      setState("subQueries", ev.payload.sub_queries.map((text) => ({ text, found: 0, done: 0 })));
      break;

    case "subquery_start":
      addLog("info", `→ ${ev.payload.sub_query}`);
      break;

    case "source_start":
      setState("sourceStatus", ev.payload.source, { phase: "querying" });
      addLog("info", `   querying ${ev.payload.source}`);
      break;

    case "source_done":
      setState("sourceStatus", ev.payload.source, {
        phase: "done",
        doneCount: ev.payload.count,
      });
      addLog("info", `   ${ev.payload.source}: +${ev.payload.count}`);
      break;

    case "source_error":
      setState("sourceStatus", ev.payload.source, { phase: "error" });
      setState(
        produce((s) => {
          s.sourceIssues = [
            ...s.sourceIssues,
            { source: ev.payload.source, error: ev.payload.error, ts: Date.now() },
          ].slice(-50);
        })
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
          s.inFlight[ev.payload.task_id] = {
            task_id: ev.payload.task_id,
            url: ev.payload.url,
            title: ev.payload.title,
            source: ev.payload.source,
            startedAt: Date.now(),
            downloaded: 0,
            total: 0,
          };
          s.active = Object.keys(s.inFlight).length;
        })
      );
      break;

    case "download_progress": {
      const current = state.inFlight[ev.payload.task_id];
      if (current) {
        const delta = Math.max(0, ev.payload.downloaded - current.downloaded);
        setState("inFlight", ev.payload.task_id, "downloaded", ev.payload.downloaded);
        setState("inFlight", ev.payload.task_id, "total", ev.payload.total);
        if (delta > 0) setState("bytesAccum", (b) => b + delta);
      }
      break;
    }

    case "download_done": {
      const item: CompletedItem = {
        task_id: ev.payload.task_id,
        url: ev.payload.url,
        title: ev.payload.title,
        source: ev.payload.source,
        status: "done",
        identifier: ev.payload.identifier,
        local_path: ev.payload.local_path,
        absolute_path: ev.payload.absolute_path,
        text_path: ev.payload.text_path,
      };
      setState(
        produce((s) => {
          delete s.inFlight[ev.payload.task_id];
          s.active = Object.keys(s.inFlight).length;
          s.done = ev.payload.done;
          s.failed = ev.payload.failed;
          s.total = ev.payload.total;
          s.completed = [...s.completed, item].slice(-500);
        })
      );
      break;
    }

    case "download_failed": {
      const item: CompletedItem = {
        task_id: ev.payload.task_id,
        url: ev.payload.url,
        title: ev.payload.title,
        source: ev.payload.source,
        status: "failed",
        identifier: ev.payload.identifier,
        error: ev.payload.error,
      };
      setState(
        produce((s) => {
          delete s.inFlight[ev.payload.task_id];
          s.active = Object.keys(s.inFlight).length;
          s.done = ev.payload.done;
          s.failed = ev.payload.failed;
          s.total = ev.payload.total;
          s.completed = [...s.completed, item].slice(-500);
        })
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
      });
      addLog(
        ev.type === "cancelled" ? "warn" : "info",
        ev.type === "cancelled"
          ? `Cancelled. Saved ${ev.payload.done} file(s).`
          : `Done. ${ev.payload.done} saved, ${ev.payload.failed} failed.`
      );
      break;

    case "filtered":
      setState("filteredCount", (prev) => prev + ev.payload.count);
      addLog("info", `   ${ev.payload.source}: filtered ${ev.payload.count} off-topic`);
      break;

    case "error":
      setState({ running: false, fatalError: ev.payload.message });
      addLog("error", `Error: ${ev.payload.message}`);
      break;
  }
}

/// 600ms ticker that converts bytesAccum into MB/s samples, keeping the
/// last 32 readings. Started lazily on the first state.running flip and
/// stopped when running flips back to false. Single-instance via the
/// `tickerId` guard so hot-reload doesn't spawn stacked timers.
let tickerId: ReturnType<typeof setInterval> | null = null;
let lastTick = 0;
let lastBytes = 0;

function startTicker() {
  if (tickerId != null) return;
  lastTick = performance.now();
  lastBytes = state.bytesAccum;
  tickerId = setInterval(() => {
    const now = performance.now();
    const dt = (now - lastTick) / 1000;
    lastTick = now;
    const bytes = state.bytesAccum;
    const delta = Math.max(0, bytes - lastBytes);
    lastBytes = bytes;
    const mbps = dt > 0 ? (delta / 1_000_000) / dt : 0;
    setState("speedHist", (prev) => {
      const next = prev.slice(prev.length >= 32 ? 1 : 0);
      next.push(mbps);
      return next;
    });
  }, 600);
}

function stopTicker() {
  if (tickerId != null) {
    clearInterval(tickerId);
    tickerId = null;
  }
}

async function startSearch(query: string) {
  if (!query.trim() || state.running) return;
  if (settings.selectedSources.length === 0) return;

  reset(query.trim());
  setState("running", true);
  startTicker();

  try {
    await api.startRun({
      query: query.trim(),
      sources: settings.selectedSources,
      out_dir: settings.libraryRoot,
      per_source: settings.perSource,
      max_total: settings.maxTotal,
      concurrency: settings.concurrency,
      extract: true,
      source_options: {
        searxng: { instance_url: settings.searxngUrl },
      },
    });
  } catch (e) {
    setState("running", false);
    stopTicker();
    apply({ type: "error", payload: { message: String(e) } });
  }
}

// Stop the ticker whenever a run ends, by either complete/cancelled/error.
// We do this here (rather than in the switch) so any path that flips
// running=false gets cleaned up.
let prevRunning = false;
setInterval(() => {
  if (prevRunning && !state.running) stopTicker();
  prevRunning = state.running;
}, 200);

async function exportZip(): Promise<ExportResult | null> {
  if (!state.folder) return null;

  const slug = state.folder.split("/").pop() ?? "library";
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
    return state.total > 0
      ? Math.round(((state.done + state.failed) / state.total) * 100)
      : 0;
  },
  get currentMbps() {
    const h = state.speedHist;
    return h.length > 0 ? h[h.length - 1] : 0;
  },
  get avgMbps() {
    const h = state.speedHist;
    if (!h.length) return 0;
    return h.reduce((s, v) => s + v, 0) / h.length;
  },
  get etaSec(): number | null {
    const remaining = Math.max(0, state.total - state.done - state.failed);
    const mbps = this.currentMbps;
    if (mbps <= 0 || remaining === 0) return null;
    // Assume ~2 MB per remaining doc as a rough average (matches the
    // bundle's heuristic). It's only displayed when running so error
    // margins are acceptable.
    return Math.round((remaining * 2_000_000) / (mbps * 1_000_000));
  },
  apply,
  startSearch,
  exportZip,
  clearFatalError() {
    setState("fatalError", null);
  },
};
