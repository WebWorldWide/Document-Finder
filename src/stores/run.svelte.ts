import { save } from "@tauri-apps/plugin-dialog";
import { api, type ExportResult } from "@/lib/tauri";
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

export class RunStore {
  running = $state(false);
  query = $state("");
  subQueries = $state<string[]>([]);
  found = $state(0);
  done = $state(0);
  failed = $state(0);
  total = $state(0);
  active = $state(0);
  filteredCount = $state(0);
  inFlight = $state<Record<string, InFlight>>({});
  completed = $state<CompletedItem[]>([]);
  sourceIssues = $state<SourceIssue[]>([]);
  log = $state<{ ts: number; level: "info" | "warn" | "error"; msg: string }[]>([]);
  folder = $state<string | null>(null);
  manifest = $state<string | null>(null);
  fatalError = $state<string | null>(null);

  overallPct = $derived(this.total > 0 ? Math.round(((this.done + this.failed) / this.total) * 100) : 0);

  reset(query: string) {
    this.running = false;
    this.query = query;
    this.subQueries = [];
    this.found = 0;
    this.done = 0;
    this.failed = 0;
    this.total = 0;
    this.active = 0;
    this.filteredCount = 0;
    this.inFlight = {};
    this.completed = [];
    this.sourceIssues = [];
    this.log = [];
    this.folder = null;
    this.manifest = null;
    this.fatalError = null;
  }

  async startSearch(query: string, settings: any) {
    if (!query.trim() || this.running) return;
    if (settings.selectedSources.length === 0) return;

    this.reset(query.trim());
    this.running = true;

    try {
      await api.startRun({
        query: query.trim(),
        sources: settings.selectedSources,
        out_dir: settings.libraryRoot,
        per_source: settings.perSource,
        max_total: settings.maxTotal,
        concurrency: settings.concurrency,
        extract: true,
        source_options: {},
      });
    } catch (e) {
      this.running = false;
      this.apply({ type: "error", payload: { message: String(e) } });
    }
  }

  async exportZip(): Promise<ExportResult | null> {
    if (!this.folder) return null;

    const slug = this.folder.split("/").pop() ?? "library";
    const dest = await save({
      defaultPath: `${slug}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });

    if (!dest) return null;

    try {
      const result = await api.exportLibraryZip(this.folder, dest);
      await api.revealInFinder(result.dest);
      return result;
    } catch (e) {
      throw e;
    }
  }

  apply(ev: DfEvent) {
    const addLog = (level: "info" | "warn" | "error", msg: string) => {
      this.log = [...this.log, { ts: Date.now(), level, msg }].slice(-200);
    };

    switch (ev.type) {
      case "keywords":
        this.subQueries = ev.payload.sub_queries;
        break;
      case "subquery_start":
        addLog("info", `→ ${ev.payload.sub_query}`);
        break;
      case "source_start":
        addLog("info", `   query ${ev.payload.source}`);
        break;
      case "source_done":
        addLog("info", `   ${ev.payload.source}: +${ev.payload.count}`);
        break;
      case "source_error":
        this.sourceIssues = [
          ...this.sourceIssues,
          {
            source: ev.payload.source,
            error: ev.payload.error,
            ts: Date.now(),
          },
        ].slice(-50);
        addLog("warn", `   ${ev.payload.source}: ${ev.payload.error}`);
        break;
      case "found":
        this.found = ev.payload.total;
        break;
      case "found_total":
        this.total = ev.payload.count;
        addLog("info", `Discovery complete — ${ev.payload.count} candidate(s)`);
        break;
      case "download_started": {
        this.inFlight[ev.payload.url] = {
          url: ev.payload.url,
          title: ev.payload.title,
          source: ev.payload.source,
          startedAt: Date.now(),
          downloaded: 0,
          total: 0,
        };
        this.active = Object.keys(this.inFlight).length;
        break;
      }
      case "download_progress": {
        const cur = this.inFlight[ev.payload.url];
        if (cur) {
          this.inFlight[ev.payload.url] = {
            ...cur,
            downloaded: ev.payload.downloaded,
            total: ev.payload.total,
          };
        }
        break;
      }
      case "download_done": {
        delete this.inFlight[ev.payload.url];
        const item: CompletedItem = {
          url: ev.payload.url,
          title: ev.payload.title,
          source: ev.payload.source,
          status: "done",
          local_path: ev.payload.local_path,
          absolute_path: ev.payload.absolute_path,
          text_path: ev.payload.text_path,
        };
        this.active = Object.keys(this.inFlight).length;
        this.done = ev.payload.done;
        this.failed = ev.payload.failed;
        this.total = ev.payload.total;
        this.completed = [...this.completed, item].slice(-500);
        break;
      }
      case "download_failed": {
        delete this.inFlight[ev.payload.url];
        const item: CompletedItem = {
          url: ev.payload.url,
          title: ev.payload.title,
          source: ev.payload.source,
          status: "failed",
          error: ev.payload.error,
        };
        this.active = Object.keys(this.inFlight).length;
        this.done = ev.payload.done;
        this.failed = ev.payload.failed;
        this.total = ev.payload.total;
        this.completed = [...this.completed, item].slice(-500);
        break;
      }
      case "complete":
      case "cancelled":
        this.running = false;
        this.folder = ev.payload.folder;
        this.manifest = ev.payload.manifest;
        this.done = ev.payload.done;
        this.failed = ev.payload.failed;
        this.total = ev.payload.total;
        addLog(
          ev.type === "cancelled" ? "warn" : "info",
          ev.type === "cancelled"
            ? `Cancelled. Saved ${ev.payload.done} file(s).`
            : `Done. ${ev.payload.done} saved, ${ev.payload.failed} failed.`
        );
        break;
      case "filtered":
        this.filteredCount += ev.payload.count;
        addLog("info", `   ${ev.payload.source}: filtered ${ev.payload.count} off-topic`);
        break;
      case "error":
        this.running = false;
        this.fatalError = ev.payload.message;
        addLog("error", `Error: ${ev.payload.message}`);
        break;
    }
  }
}

export const runStore = new RunStore();
