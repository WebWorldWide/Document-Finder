import { createStore, produce, reconcile } from "solid-js/store";
import type { DfEvent } from "@/lib/events";
import { api, type Document } from "@/lib/tauri";
import { settings } from "@/stores/settings";
import { uiStore } from "@/stores/ui";

export type RunStatus = "idle" | "running" | "stopping" | "done" | "cancelled" | "error";
export type StageId = "search" | "download" | "read";
export type StageState = "waiting" | "active" | "done";

export interface SavedDoc {
  /** Catalog number in arrival order (1-based). */
  n: number;
  doc: Document;
  path: string;
  textPath?: string;
  bytes: number;
  cached: boolean;
  extractError?: string;
}

export interface InFlight {
  url: string;
  title: string;
  source: string;
  downloaded: number;
  total: number;
}

export interface Failure {
  doc: Document;
  error: string;
}

export interface SourceStat {
  found: number;
  error?: string;
}

/** One finished download, in arrival order (drives the tile map). */
export interface Outcome {
  ok: boolean;
  source: string;
  title: string;
}

/** Saved docs kept in memory for the live ledger (the Library has all). */
const MAX_SAVED_IN_MEMORY = 2000;

interface State {
  status: RunStatus;
  /** True while this run is a "retry failed" (no discovery). */
  retry: boolean;
  query: string;
  startedAt: number;
  endedAt: number;
  found: number;
  planned: number;
  done: number;
  failed: number;
  offTopic: number;
  subQueries: string[];
  sources: Record<string, SourceStat>;
  discoveryDone: boolean;
  inFlight: InFlight[];
  saved: SavedDoc[];
  failures: Failure[];
  /** Every finished download (saved or failed) in the order it finished. */
  outcomes: Outcome[];
  /** Download speed in bytes/s, sampled once a second while running. */
  speedSeries: number[];
  bytes: number;
  folder: string;
  error: string;
}

const initial = (): State => ({
  status: "idle",
  retry: false,
  query: "",
  startedAt: 0,
  endedAt: 0,
  found: 0,
  planned: 0,
  done: 0,
  failed: 0,
  offTopic: 0,
  subQueries: [],
  sources: {},
  discoveryDone: false,
  inFlight: [],
  saved: [],
  failures: [],
  outcomes: [],
  speedSeries: [],
  bytes: 0,
  folder: "",
  error: "",
});

/** Speed samples kept for the chart (2 minutes at 1/s). */
const MAX_SPEED_SAMPLES = 120;
let sampler: ReturnType<typeof setInterval> | undefined;
let lastBytes = 0;

function startSampler() {
  stopSampler();
  lastBytes = 0;
  sampler = setInterval(() => {
    const b = state.bytes;
    const delta = Math.max(0, b - lastBytes);
    lastBytes = b;
    setState(
      produce((s) => {
        s.speedSeries.push(delta);
        if (s.speedSeries.length > MAX_SPEED_SAMPLES) s.speedSeries.shift();
      }),
    );
  }, 1000);
}

function stopSampler() {
  if (sampler) clearInterval(sampler);
  sampler = undefined;
}

const [state, setState] = createStore<State>(initial());

function docOf(p: Partial<Document> & { title: string; url: string; source: string }): Document {
  return {
    title: p.title,
    url: p.url,
    source: p.source,
    authors: p.authors ?? [],
    year: p.year,
    abstract: p.abstract,
    identifier: p.identifier,
    doi: p.doi,
    alt_urls: p.alt_urls,
    cited_by: p.cited_by,
    venue: p.venue,
    license: p.license,
  };
}

function finish(status: RunStatus, folder?: string) {
  stopSampler();
  setState(
    produce((s) => {
      s.status = status;
      s.endedAt = Date.now();
      s.inFlight = [];
      s.discoveryDone = true;
      if (folder) s.folder = folder;
    }),
  );
}

/** Reduce one backend event into run state. Exported for tests. */
export function apply(ev: DfEvent) {
  if (state.status === "idle" && ev.type !== "error") return;
  switch (ev.type) {
    case "keywords":
      setState("subQueries", ev.payload.sub_queries);
      break;
    case "found":
      setState(
        produce((s) => {
          s.found = Math.max(s.found, ev.payload.total);
          // meta_search results arrive as "meta_search/<engine>".
          const src = ev.payload.source.split("/")[0];
          s.sources[src] = {
            ...(s.sources[src] ?? { found: 0 }),
            found: (s.sources[src]?.found ?? 0) + 1,
          };
        }),
      );
      break;
    case "found_total":
      setState(
        produce((s) => {
          s.found = ev.payload.count;
          s.discoveryDone = true;
        }),
      );
      break;
    case "source_error":
      setState("sources", ev.payload.source, (prev) => ({
        found: prev?.found ?? 0,
        error: ev.payload.kind,
      }));
      break;
    case "plan":
      setState("planned", ev.payload.planned);
      break;
    case "ranking_done":
      setState("offTopic", ev.payload.rejected);
      break;
    case "download_started":
      setState(
        produce((s) => {
          if (!s.inFlight.some((f) => f.url === ev.payload.url)) {
            s.inFlight.unshift({ ...ev.payload, downloaded: 0, total: 0 });
            if (s.inFlight.length > 40) s.inFlight.length = 40;
          }
        }),
      );
      break;
    case "download_progress": {
      const i = state.inFlight.findIndex((f) => f.url === ev.payload.url);
      if (i >= 0) {
        setState("inFlight", i, { downloaded: ev.payload.downloaded, total: ev.payload.total });
      }
      break;
    }
    case "download_done":
      setState(
        produce((s) => {
          const p = ev.payload;
          s.inFlight = s.inFlight.filter((f) => f.url !== p.url);
          s.done = p.done;
          s.failed = p.failed;
          s.planned = Math.max(s.planned, p.total, p.done + p.failed);
          if (!p.cached) s.bytes += p.bytes;
          s.outcomes.push({ ok: true, source: p.source.split("/")[0], title: p.title });
          s.saved.unshift({
            n: p.done,
            doc: docOf(p),
            path: p.absolute_path,
            textPath: p.text_path,
            bytes: p.bytes,
            cached: p.cached,
            extractError: p.extract_error,
          });
          if (s.saved.length > MAX_SAVED_IN_MEMORY) s.saved.length = MAX_SAVED_IN_MEMORY;
        }),
      );
      break;
    case "download_failed":
      setState(
        produce((s) => {
          const p = ev.payload;
          s.inFlight = s.inFlight.filter((f) => f.url !== p.url);
          s.done = p.done;
          s.failed = p.failed;
          s.planned = Math.max(s.planned, p.total, p.done + p.failed);
          s.failures.push({ doc: docOf(p), error: p.error });
          s.outcomes.push({ ok: false, source: p.source.split("/")[0], title: p.title });
        }),
      );
      break;
    case "complete":
      setState(
        produce((s) => {
          s.done = ev.payload.done;
          s.failed = ev.payload.failed;
          s.planned = Math.max(ev.payload.total, ev.payload.done + ev.payload.failed);
        }),
      );
      finish("done", ev.payload.folder);
      uiStore.announce(`Search finished. ${ev.payload.done} documents saved.`);
      break;
    case "cancelled":
      setState("done", ev.payload.done);
      finish("cancelled", ev.payload.folder);
      uiStore.announce("Search stopped.");
      break;
    case "error":
      setState(
        produce((s) => {
          s.error = ev.payload.message;
        }),
      );
      finish("error");
      break;
    default:
      break;
  }
}

/** The three plain-language stages shown in the run view. */
export function stages(): Record<StageId, StageState> {
  const live = state.status === "running" || state.status === "stopping";
  const ended = !live && state.status !== "idle";
  const search: StageState = state.retry
    ? "done"
    : state.discoveryDone || ended
      ? "done"
      : "active";
  const download: StageState =
    ended ||
    (state.discoveryDone &&
      state.inFlight.length === 0 &&
      state.done + state.failed >= state.planned &&
      state.planned > 0)
      ? "done"
      : state.inFlight.length > 0 || state.done + state.failed > 0
        ? "active"
        : "waiting";
  const read: StageState = ended ? "done" : state.done > 0 ? "active" : "waiting";
  return { search, download, read };
}

export function progressFraction(): number {
  if (state.status === "done") return 1;
  const total = state.planned;
  if (total <= 0) return 0;
  return Math.min(1, (state.done + state.failed) / total);
}

async function start(query: string, retry: Failure[] | null = null) {
  const folder = state.folder;
  setState(reconcile(initial()));
  setState(
    produce((s) => {
      s.status = "running";
      s.retry = !!retry;
      s.query = query;
      s.startedAt = Date.now();
      if (retry) {
        s.folder = folder;
        s.planned = retry.length;
      }
    }),
  );
  startSampler();
  try {
    if (retry) {
      await api.retryRun({ folder, query, docs: retry.map((f) => f.doc) });
    } else {
      await api.startRun({
        query,
        sources: [...settings.selectedSources],
        out_dir: settings.libraryRoot,
      });
    }
  } catch (e) {
    setState("error", String(e));
    finish("error");
  }
}

export const runStore = {
  state,
  apply,
  start: (query: string) => start(query),
  retryFailed: () => {
    const failures = state.failures.filter((f) => !/Same file as/.test(f.error));
    if (failures.length > 0 && state.folder) return start(state.query, failures);
    return Promise.resolve();
  },
  async stop() {
    if (state.status !== "running") return;
    setState("status", "stopping");
    try {
      await api.cancelRun();
    } catch (e) {
      console.error("cancel failed", e);
    }
  },
  reset() {
    stopSampler();
    setState(reconcile(initial()));
  },
  get live() {
    return state.status === "running" || state.status === "stopping";
  },
};

/** Test hook: put the store in "running" without calling the backend. */
export function __beginForTest(query: string) {
  setState(reconcile(initial()));
  setState(
    produce((s) => {
      s.status = "running";
      s.query = query;
      s.startedAt = Date.now();
    }),
  );
}
