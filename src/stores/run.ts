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

interface RunState {
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
  log: LogEntry[];
  folder: string | null;
  manifest: string | null;
  fatalError: string | null;
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
  log: [],
  folder: null,
  manifest: null,
  fatalError: null,
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
    log: [],
    folder: null,
    manifest: null,
    fatalError: null,
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
      addLog("info", `   querying ${ev.payload.source}`);
      break;

    case "source_done":
      addLog("info", `   ${ev.payload.source}: +${ev.payload.count}`);
      break;

    case "source_error":
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

    case "download_progress":
      if (state.inFlight[ev.payload.task_id]) {
        setState("inFlight", ev.payload.task_id, "downloaded", ev.payload.downloaded);
        setState("inFlight", ev.payload.task_id, "total", ev.payload.total);
      }
      break;

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

async function startSearch(query: string) {
  if (!query.trim() || state.running) return;
  if (settings.selectedSources.length === 0) return;

  reset(query.trim());
  setState("running", true);

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
    apply({ type: "error", payload: { message: String(e) } });
  }
}

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
  apply,
  startSearch,
  exportZip,
  clearFatalError() {
    setState("fatalError", null);
  },
};
