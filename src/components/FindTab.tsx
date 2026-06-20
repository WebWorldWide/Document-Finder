import {
  createSignal,
  createMemo,
  createEffect,
  onCleanup,
  untrack,
  Show,
  For,
  Index,
} from "solid-js";
import {
  Search,
  Square,
  Sparkles,
  Archive,
  FolderOpen,
  BookOpen,
  Loader2,
  RotateCw,
} from "lucide-solid";
import DocRow, { type StreamDoc } from "./DocRow";
import SourcePanel from "./SourcePanel";
import Sparkline from "./Sparkline";
import Banner from "./Banner";
import ModelStatusBadge from "./ModelStatusBadge";
import { confirm } from "@tauri-apps/plugin-dialog";
import { runStore } from "@/stores/run";
import { pipelineStore } from "@/stores/pipeline";
import type { PipelineStage } from "@/lib/events";
import { settings, toggleSource, setSettings, saveSettings } from "@/stores/settings";
import { uiStore } from "@/stores/ui";
import { streamLayout } from "@/stores/theme";
import { api } from "@/lib/tauri";
import {
  ALL_SOURCES,
  META_SEARCH_COVERED,
  SOURCE_LABELS,
  sourceColor,
  sourceLabel,
  formatBytes,
  ftypeFromPath,
  type SourceId,
} from "@/lib/utils";
import { humanizeDownloadError, humanizeSourceKind, issueKindTag } from "@/lib/errors";

// The standalone `searxng` source is NOT shown as a user-facing toggle: it's
// meta_search's own in-process pool fallback, and when meta_search (the default)
// is on the backend drops searxng as already-covered — so a checked searxng row
// does nothing. Excluding it from the grid keeps the "N of N" count honest
// (otherwise "Enable all" could never reach all rows, since it skipped searxng).
const PRIMARY_SOURCES: SourceId[] = ALL_SOURCES.filter(
  (s) => !META_SEARCH_COVERED.includes(s) && s !== "searxng",
);
// Bulk "Enable all" / "Invert" act on the same visible set as the grid + count.
const BULK_SOURCES: SourceId[] = PRIMARY_SOURCES;

const STAGE_LABEL: Record<string, string> = {
  llm_expand: "Expand",
  discovery: "Discover",
  rank: "Rank",
  semantic_rerank: "Rerank",
  llm_filter: "Filter",
  citation_enrich: "Cite",
  download: "Download",
  extract: "Extract",
};

function laneKey(source: string): string {
  if (source.startsWith("meta_search")) return "meta_search";
  if (META_SEARCH_COVERED.includes(source as SourceId)) return "meta_search";
  // Pool-fallback docs (all web circuits open) arrive as searxng_local/pool —
  // fold them onto whichever aggregator the run used: meta_search, or the
  // standalone `searxng` source when that's what's enabled.
  if (source === "searxng_local" || source === "searxng_pool") {
    return runStore.state.sources.includes("meta_search") ? "meta_search" : "searxng";
  }
  return source;
}

// Throughput-sampler window: SPEED_WINDOW samples taken every
// SPEED_INTERVAL_MS, so the rolling "avg" covers SPEED_WINDOW_SEC of history.
// Kept as named constants so the "avg Ns" label can't drift from the real
// window if either is retuned.
const SPEED_WINDOW = 32;
const SPEED_INTERVAL_MS = 600;
const SPEED_WINDOW_SEC = Math.round((SPEED_WINDOW * SPEED_INTERVAL_MS) / 1000);

// macOS uses ⌘ as the modifier; the submit handler accepts Cmd OR Ctrl, but the
// on-screen hint should show the key the user actually reaches for.
const IS_MAC =
  typeof navigator !== "undefined" &&
  /Mac|iP(hone|ad|od)/.test(navigator.platform || navigator.userAgent || "");
const MOD_KEY = IS_MAC ? "⌘" : "Ctrl";

export default function FindTab() {
  const [query, setQuery] = createSignal("");
  const [exporting, setExporting] = createSignal(false);
  const [exportedTo, setExportedTo] = createSignal<string | null>(null);
  const [exportError, setExportError] = createSignal<string | null>(null);
  const [openError, setOpenError] = createSignal<string | null>(null);
  const [speedHist, setSpeedHist] = createSignal<number[]>([]);
  // How many real throughput samples have been pushed this run, so the average
  // isn't diluted by the 32 seed zeros at the start (which made the early ETA
  // wildly pessimistic).
  const [samplesSeen, setSamplesSeen] = createSignal(0);
  const [stopping, setStopping] = createSignal(false);

  const rs = () => runStore.state;
  // Accessor (not a one-time snapshot) so the header stats stay reactive as
  // libraries load — `lifetimeStats` is a getter that recomputes from a signal.
  const stats = () => uiStore.lifetimeStats;

  // Reset the local "Stopping…" latch once the backend confirms the run ended
  // (both EV_COMPLETE and EV_CANCELLED set running=false in the run store).
  createEffect(() => {
    if (!rs().running) setStopping(false);
  });

  // Announce fatal errors to screen readers through the always-mounted region.
  // The visible error Banner is mounted together with its text (an unreliable
  // pattern for live regions), and liveStatus() is empty for errors that happen
  // before a run folder exists (offline pre-check, concurrent-run guard) — so an
  // SR user would otherwise get no spoken feedback that the search failed.
  createEffect(() => {
    const err = rs().fatalError;
    if (err) uiStore.announce(humanizeDownloadError(err));
  });

  const hasRun = createMemo(
    () =>
      rs().running ||
      rs().completed.length > 0 ||
      Object.keys(rs().inFlight).length > 0 ||
      // A finished run that found candidates but downloaded none (everything was
      // ranked off-topic) still has a story to tell — show the card so the
      // found/off-topic stats explain why nothing was saved. (noResults() is
      // found===0, so the two states stay mutually exclusive.)
      (rs().folder !== null && rs().found > 0),
  );

  // ---- Live throughput sampler — samples cumulative bytes every 600ms while a
  // run is active and pushes MB/s into a rolling window for the sparkline.
  createEffect(() => {
    if (!rs().running) return;
    let lastBytes = untrack(() => runStore.state.bytesDownloaded);
    let lastT = performance.now();
    setSpeedHist(new Array(SPEED_WINDOW).fill(0));
    setSamplesSeen(0);
    const timer = window.setInterval(() => {
      const now = performance.now();
      const dt = (now - lastT) / 1000;
      const b = runStore.state.bytesDownloaded;
      const mbps = dt > 0 ? (b - lastBytes) / 1_000_000 / dt : 0;
      lastBytes = b;
      lastT = now;
      setSpeedHist((h) => [...h.slice(1), Math.max(0, mbps)]);
      setSamplesSeen((n) => n + 1);
    }, SPEED_INTERVAL_MS);
    onCleanup(() => clearInterval(timer));
  });

  const currentMbps = () => {
    const h = speedHist();
    return h.length ? h[h.length - 1] : 0;
  };
  // Average over only the REAL samples taken so far (the window is pre-seeded
  // with zeros), so a half-full window doesn't report half the true rate.
  const avgMbps = () => {
    const h = speedHist();
    const n = Math.min(samplesSeen(), h.length);
    if (n <= 0) return 0;
    const recent = h.slice(h.length - n);
    return recent.reduce((a, b) => a + b, 0) / n;
  };
  // Average observed file size this run (MB), used for ETA. Divides network
  // bytes by the count of files that actually transferred (cached files add to
  // `done` but contribute 0 bytes, which would bias the average low). Falls back
  // to ~2 MB before anything has completed over the network.
  const networkDone = () => rs().completed.filter((c) => c.status === "done" && !c.cached).length;
  const avgDocMb = () => {
    const n = networkDone();
    return n > 0 ? Math.max(0.1, rs().bytesDownloaded / 1_000_000 / n) : 2;
  };
  const downloadsStarted = () => {
    const st = pipelineStore.stages.download.state;
    return st === "started" || st === "progress" || st === "done";
  };
  // The DOWNLOAD scope (kept docs), not the pre-rank candidate count. runStore's
  // `total` is the merged-candidate count until the first download_done event
  // reconciles it, so during the download-start window it's ~10x too big. The
  // download stage event already carries the real kept total — prefer it once
  // downloads start so the progress bar and ETA don't lurch (bar to 0%, ETA to a
  // wildly pessimistic value) before the first file lands.
  const downloadTotal = () => {
    const t = pipelineStore.stages.download.total;
    return downloadsStarted() && t != null && t > 0 ? t : rs().total;
  };
  const etaSec = createMemo(() => {
    const remaining = Math.max(0, downloadTotal() - rs().done - rs().failed);
    // Use the smoothed window average, not the volatile last 600ms sample, so a
    // single zero/spike sample doesn't make the ETA vanish or collapse.
    const mbps = avgMbps();
    if (remaining <= 0 || mbps <= 0) return null;
    return Math.round((remaining * avgDocMb()) / mbps);
  });
  const fmtEta = (s: number | null) =>
    s == null ? "—" : s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;

  const fileTypeCounts = createMemo(() => {
    const c: Record<string, number> = { pdf: 0, epub: 0, html: 0, txt: 0 };
    for (const it of rs().completed) {
      if (it.status === "done") {
        const ft = ftypeFromPath(it.local_path);
        if (ft) c[ft] += 1;
      }
    }
    return c;
  });

  // ---- Current pipeline phase. Drives a live status line + the progress bar
  // so the run card keeps moving during the long pre-download rank phase —
  // otherwise it sits at 0 saved / 0% and looks frozen while ranking runs.
  const activePhase = createMemo(() => {
    const stages = pipelineStore.stages;
    let active: PipelineStage | null = null;
    for (const s of pipelineStore.ordered) {
      const st = stages[s].state;
      if (st === "started" || st === "progress") active = s; // rightmost in-flight stage
    }
    if (!active) return null;
    const e = stages[active];
    return { label: STAGE_LABEL[active] ?? active, count: e.count, total: e.total };
  });

  // Whether the "By source" lanes should show SAVED counts vs FOUND counts. The
  // backend emits a download stage even when 0 candidates are kept, so
  // downloadsStarted() alone would flip an all-off-topic finish to all-zero
  // saved bars that contradict the "found N" headline — require something
  // actually saved/in-flight before switching to the saved view.
  const showSaved = () => downloadsStarted() && (rs().done > 0 || rs().active > 0);

  // Bar tracks download completion once downloads begin; before that it tracks
  // the active stage's progress (e.g. ranking 181/500) so it visibly advances.
  const barPct = createMemo(() => {
    // A finished, non-cancelled run reads as done — fill the bar to 100% even in
    // the all-off-topic case (download total is 0, so overallPct would be 0 and
    // the bar would look stuck/empty on a card that has actually finished).
    if (!rs().running && rs().folder && !rs().cancelled) return 100;
    if (downloadsStarted()) {
      const t = downloadTotal();
      return t > 0 ? Math.min(100, Math.round(((rs().done + rs().failed) / t) * 100)) : 0;
    }
    const p = activePhase();
    if (p && p.total) return Math.min(100, Math.round(((p.count ?? 0) / p.total) * 100));
    return runStore.overallPct;
  });

  // Short label for what the pipeline is doing right now (null once downloads
  // start — the saved/found stats tell that story from there).
  const phaseLabel = createMemo(() => {
    if (downloadsStarted()) return null;
    const p = activePhase();
    if (!p) return rs().running ? "Working…" : null;
    return p.total ? `${p.label} ${p.count ?? 0}/${p.total}` : `${p.label}…`;
  });

  // Coarse status for screen readers. The visible stats block updates many times
  // per second (found counter, per-tick phase count), so it is NOT a live region
  // — that would flood assistive tech. This announces only stage transitions and
  // the final summary, which change infrequently.
  const liveStatus = createMemo(() => {
    if (rs().running) {
      if (downloadsStarted()) return "Downloading documents…";
      const p = activePhase();
      return p ? `${p.label}…` : "Working…";
    }
    if (rs().folder)
      return `Search ${rs().cancelled ? "cancelled" : "complete"} — ${rs().done} saved, ${rs().failed} failed`;
    return "";
  });

  const lanes = createMemo(() => {
    const totals: Record<string, { found: number; done: number; inflight: number }> = {};
    for (const id of PRIMARY_SOURCES) totals[id] = { found: 0, done: 0, inflight: 0 };
    // `found` comes from the live per-source discovery counter so the bars
    // climb as documents are found (not just as they download). sourceStats is
    // keyed by the aggregator id (meta_search), which matches PRIMARY_SOURCES.
    const stats = rs().sourceStats;
    for (const id of PRIMARY_SOURCES) totals[id].found = stats[id]?.hits ?? 0;
    for (const it of rs().completed) {
      if (it.status === "done") {
        const k = laneKey(it.source);
        if (totals[k]) totals[k].done += 1;
      }
    }
    for (const url in rs().inFlight) {
      const k = laneKey(rs().inFlight[url].source);
      if (totals[k]) totals[k].inflight += 1;
    }
    return totals;
  });
  // The run card's lanes track the source set the RUN used (snapshot in
  // runStore), not the live selection — so re-toggling sources after a run can't
  // reshape the finished run's graph. Falls back to the live selection before a
  // run has started (so the pre-run panel preview still renders).
  const laneSources = () => {
    const snap = rs().sources.length > 0 ? rs().sources : settings.selectedSources;
    return PRIMARY_SOURCES.filter((s) => snap.includes(s));
  };
  // Before downloads begin, the bars track per-source FOUND counts (live
  // discovery); once downloads start, they track saved + in-flight files.
  const laneMax = () => {
    const ls = laneSources();
    if (showSaved()) return Math.max(1, ...ls.map((s) => lanes()[s].done + lanes()[s].inflight));
    return Math.max(1, ...ls.map((s) => lanes()[s].found));
  };

  const inFlightDocs = (): StreamDoc[] =>
    Object.values(rs().inFlight).map((d) => ({
      source: d.source,
      title: d.title,
      downloaded: d.downloaded,
      total: d.total,
      ftype: ftypeFromPath(d.url),
    }));
  // In-flight rows keyed by URL (a stable identity) rather than by list
  // position. Downloads complete out of order, so an <Index> (position-keyed)
  // reassigned each DOM row to a different file as the list reshuffled — a row's
  // title/progress visibly swapped and the bar appeared to jump backward. <For>
  // over the URL list gives each file a stable row for its whole lifecycle; the
  // per-row data is looked up reactively so progress still updates live.
  const inFlightUrls = () => Object.keys(rs().inFlight);
  const renderInFlightRows = () => (
    <For each={inFlightUrls()}>
      {(url) => (
        <Show when={rs().inFlight[url]}>
          {(d) => (
            <DocRow
              doc={{
                source: d().source,
                title: d().title,
                downloaded: d().downloaded,
                total: d().total,
                ftype: ftypeFromPath(d().url),
              }}
              kind="in-flight"
            />
          )}
        </Show>
      )}
    </For>
  );
  const SAVED_SHOWN = 40;
  const savedDocs = (): StreamDoc[] =>
    rs()
      .completed.filter((c) => c.status === "done")
      .slice(-SAVED_SHOWN)
      .reverse()
      .map((c) => ({
        source: c.source,
        title: c.title,
        status: "done" as const,
        ftype: ftypeFromPath(c.local_path),
        // Carry the on-disk size so DocRow can render it next to the checkmark.
        bytes: c.bytes,
        // Absolute path so the row can open the document in its default app.
        path: c.absolute_path,
      }));
  // Open a saved document in the OS default app — the in-app path to actually
  // READ the paper the user just found. Failure (file moved/deleted) is announced.
  const openSaved = (d: StreamDoc) => {
    if (!d.path) return;
    setOpenError(null);
    api.openPath(d.path).catch((e) => {
      // Visible banner (not just the SR announce) — the row says "click to open",
      // so a sighted user needs to see why nothing happened (e.g. file moved).
      const msg = `Couldn't open the file: ${String(e)}`;
      setOpenError(msg);
      uiStore.announce(msg);
    });
  };
  // The saved stream caps at SAVED_SHOWN rows; surface the remainder (with a
  // jump to the full Library) so "Saved 120" above 40 rows doesn't look like 80
  // files vanished.
  const savedOverflow = () => Math.max(0, rs().done - savedDocs().length);
  const renderSavedOverflow = () => (
    <Show when={savedOverflow() > 0}>
      <button
        class="df-btn sm ghost"
        style={{ "margin-top": "6px" }}
        onClick={() => void handleOpenLibrary()}
      >
        +{savedOverflow()} more — open Library
      </button>
    </Show>
  );

  const issues = createMemo(() => {
    const out: { source?: string; label: string; tag?: string; text: string }[] = [];
    for (const i of rs().sourceIssues) {
      out.push({
        source: i.source,
        // sourceLabel() strips the meta_search/<engine> prefix → "DuckDuckGo",
        // not the raw "meta_search/web", matching the colored dot beside it.
        label: sourceLabel(i.source),
        tag: issueKindTag(i.kind) + (i.count > 1 ? ` ×${i.count}` : ""),
        text: humanizeSourceKind(i.kind, i.source),
      });
    }
    for (const f of rs().completed.filter((c) => c.status === "failed")) {
      out.push({
        source: f.source,
        label: sourceLabel(f.source),
        text: `${f.title.slice(0, 48)} — ${humanizeDownloadError(f.error)}`,
      });
    }
    // Files that saved to disk but whose library index row failed to write:
    // the bytes are there, but they won't show up in the Library view.
    for (const c of rs().completed) {
      if (c.status === "done" && c.indexError) {
        out.push({
          source: c.source,
          label: sourceLabel(c.source),
          tag: "not indexed",
          text: `${c.title.slice(0, 48)} — saved to disk but not added to the library index`,
        });
      }
    }
    return out;
  });

  // ---- source toggles ----
  function enableAll() {
    setSettings("selectedSources", (prev) => [...new Set([...prev, ...BULK_SOURCES])]);
    saveSettings();
  }
  function disableAll() {
    setSettings("selectedSources", (prev) => prev.filter((s) => !PRIMARY_SOURCES.includes(s)));
    saveSettings();
  }
  function invert() {
    setSettings("selectedSources", (prev) => {
      const set = new Set(prev);
      for (const s of BULK_SOURCES) {
        if (set.has(s)) set.delete(s);
        else set.add(s);
      }
      return [...set];
    });
    saveSettings();
  }

  async function handleSearch() {
    if (!query().trim() || rs().running || settings.selectedSources.length === 0) return;
    setStopping(false); // clear any stranded "Stopping…" latch from a prior run
    setExportedTo(null);
    // Also clear a prior export ERROR — otherwise a stale red "Export failed"
    // banner floats above the new run for its whole duration.
    setExportError(null);
    await runStore.startSearch(query());
  }
  // Confirm a Stop only when downloads are actively in flight — cancelling then
  // discards completed network work. A stop during discovery is cheap, so it's
  // not gated.
  async function requestStop() {
    if (stopping()) return;
    if (rs().active > 0) {
      const ok = await confirm(
        `Stop now? ${rs().active} download${rs().active === 1 ? "" : "s"} in progress will be cancelled.`,
        { title: "Stop search", kind: "warning" },
      );
      if (!ok) return;
      // The run may have finished naturally while the confirm dialog was open —
      // don't latch "Stopping…" on an already-ended run (the reset effect won't
      // fire again, and the latch would strand the NEXT run's Stop button).
      if (!rs().running) return;
    }
    setStopping(true);
    void api.cancelRun();
  }
  // Files saved this run whose text couldn't be extracted (e.g. scanned PDFs).
  const noTextCount = createMemo(
    () => rs().completed.filter((c) => c.status === "done" && c.extractError).length,
  );
  // A finished run that produced nothing — folder is set (a run happened) but
  // no candidates were found and nothing downloaded. Excludes the cancelled case
  // (handled separately) so a user Stop isn't blamed as "no documents found".
  const noResults = createMemo(
    () =>
      !rs().running &&
      !rs().cancelled &&
      rs().folder !== null &&
      rs().found === 0 &&
      rs().completed.length === 0 &&
      !rs().fatalError,
  );
  // The user Stopped before anything was found/saved — a deliberate outcome, so
  // show a neutral note instead of the "no documents found, try broader terms"
  // banner (which would wrongly blame their query/connection).
  const cancelledEmpty = createMemo(
    () =>
      !rs().running &&
      rs().cancelled &&
      rs().found === 0 &&
      rs().completed.length === 0 &&
      !rs().fatalError,
  );
  // A finished run that FOUND candidates but the ranker rejected them all as
  // off-topic (found > 0, yet nothing saved or failed). noResults() requires
  // found === 0 so it doesn't fire here — without this the user gets a "Done ·
  // saved 0" card and only a buried in-column note, with no next step.
  const allOffTopic = createMemo(
    () =>
      !rs().running &&
      !rs().cancelled &&
      rs().folder !== null &&
      rs().found > 0 &&
      rs().done === 0 &&
      rs().failed === 0 &&
      !rs().fatalError,
  );
  async function handleExport() {
    setExporting(true);
    setExportError(null);
    try {
      const result = await runStore.exportZip();
      if (result) {
        setExportedTo(result.dest);
        uiStore.announce(`Exported to ${result.dest}`);
      }
    } catch (e) {
      // Was silently swallowed with a misleading "surfaced elsewhere" comment;
      // export errors have no other surface, so show them here.
      const msg = humanizeDownloadError(String(e));
      setExportError(msg);
      uiStore.announce(`Export failed. ${msg}`);
    } finally {
      setExporting(false);
    }
  }
  async function handleOpenLibrary() {
    const folder = rs().folder;
    if (!folder) return;
    // setActiveLibrary is optional metadata (highlights the active card); the
    // Library view re-lists from disk on mount and surfaces its own error. So
    // navigate REGARDLESS of whether open_library succeeds — gating navigation on
    // it made the "Library" / "+N more" buttons silently dead when it failed
    // (e.g. the db was just purged), with no feedback at all.
    try {
      const info = await api.openLibrary(folder);
      uiStore.setActiveLibrary(info);
    } catch (e) {
      uiStore.announce(`Couldn't open that library: ${String(e)}`);
    }
    uiStore.setView("library");
  }

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <div>
          <div class="df-eyebrow">Discover</div>
          <h1 class="df-canvas-title">Discover</h1>
        </div>
        <div class="df-headstats">
          <div class="df-headstat">
            <span class="df-headstat-num">{stats().count}</span>
            <span class="df-headstat-label">libraries</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{stats().totalDocs}</span>
            <span class="df-headstat-label">docs saved</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{formatBytes(stats().totalBytes)}</span>
            <span class="df-headstat-label">on disk</span>
          </div>
        </div>
      </div>

      <div class="df-canvas-body" style={{ "padding-top": "24px" }}>
        {/* Coarse SR announcer (visually hidden), always mounted so it announces
            even the no-results completion (the run card — and its old in-card
            announcer — isn't rendered when a run finds nothing). See liveStatus. */}
        <div
          role="status"
          aria-live="polite"
          style={{
            position: "absolute",
            width: "1px",
            height: "1px",
            overflow: "hidden",
            clip: "rect(0 0 0 0)",
            "white-space": "nowrap",
          }}
        >
          {liveStatus()}
        </div>
        {/* HERO QUERY */}
        <div class="df-query-wrap">
          <textarea
            class="df-query-input"
            rows={2}
            placeholder="What are you researching?"
            aria-label="Search query"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                void handleSearch();
              }
            }}
          />
          <div class="df-query-bar">
            <span class="df-query-hint">
              <Sparkles size={12} /> Natural language — we&rsquo;ll split it into sub-queries
            </span>
            <span style={{ flex: 1 }} />
            <span class="df-query-hint">
              <span class="df-kbd">{MOD_KEY}</span>
              <span class="df-kbd">↵</span> to search
            </span>
            <Show
              when={rs().running}
              fallback={
                <button
                  class="df-btn accent"
                  onClick={() => void handleSearch()}
                  disabled={!query().trim() || settings.selectedSources.length === 0}
                >
                  <Search size={13} /> Find documents
                </button>
              }
            >
              <button
                class="df-btn danger"
                disabled={stopping()}
                onClick={() => void requestStop()}
              >
                <Square size={12} /> {stopping() ? "Stopping…" : "Stop"}
              </button>
            </Show>
          </div>
        </div>

        {/* SOURCES */}
        <div style={{ "margin-top": "18px" }}>
          <div class="df-section-head" style={{ "margin-top": 0 }}>
            <span class="df-section-label">Sources</span>
            <span class="df-section-meta">Toggle which open-access platforms to fan out to.</span>
          </div>
          <SourcePanel
            sources={PRIMARY_SOURCES}
            enabled={settings.selectedSources}
            running={rs().running}
            stats={rs().sourceStats}
            onToggle={(id) => toggleSource(id as SourceId)}
            onEnableAll={enableAll}
            onDisableAll={disableAll}
            onInvert={invert}
          />

          {/* advanced web engines */}
          <details style={{ "margin-top": "8px" }}>
            <summary
              style={{
                cursor: "pointer",
                "font-size": "11px",
                "font-weight": 500,
                color: "var(--ink-3)",
                padding: "4px 2px",
              }}
            >
              Individual web engines (advanced)
            </summary>
            <div
              style={{
                display: "flex",
                "flex-wrap": "wrap",
                gap: "6px",
                "margin-top": "8px",
              }}
            >
              <For each={META_SEARCH_COVERED}>
                {(src) => {
                  const on = () => settings.selectedSources.includes(src);
                  return (
                    <button
                      class="df-btn sm df-web-engine"
                      classList={{ on: on() }}
                      onClick={() => toggleSource(src)}
                      disabled={rs().running}
                      aria-pressed={on()}
                      // Identity color via a CSS var so the .on text/border/fill
                      // (and a midnight lift) live in globals.css — the old inline
                      // `color: sourceColor()` was dark-on-dark in midnight.
                      style={{ "--src-color": sourceColor(src) }}
                    >
                      {SOURCE_LABELS[src]}
                    </button>
                  );
                }}
              </For>
            </div>
          </details>

          <Show when={settings.selectedSources.length === 0}>
            <p style={{ "margin-top": "10px", "font-size": "12px", color: "var(--bad-ink)" }}>
              Enable at least one source to start a search.
            </p>
          </Show>
        </div>

        {/* FATAL ERROR */}
        <Show when={rs().fatalError}>
          <div style={{ "margin-top": "16px" }}>
            <Banner kind="bad" onDismiss={() => runStore.clearFatalError()}>
              <strong>Something went wrong.</strong> {humanizeDownloadError(rs().fatalError)}
            </Banner>
          </div>
        </Show>

        {/* EXPORT SUCCESS */}
        <Show when={exportedTo()}>
          <div style={{ "margin-top": "16px" }}>
            <Banner kind="ok" onDismiss={() => setExportedTo(null)}>
              Exported to <code>{exportedTo()}</code>
            </Banner>
          </div>
        </Show>

        {/* EXPORT ERROR */}
        <Show when={exportError()}>
          <div style={{ "margin-top": "16px" }}>
            <Banner kind="bad" onDismiss={() => setExportError(null)}>
              <strong>Export failed.</strong> {exportError()}
            </Banner>
          </div>
        </Show>

        {/* OPEN-DOCUMENT ERROR — a saved row's file couldn't be opened. */}
        <Show when={openError()}>
          <div style={{ "margin-top": "16px" }}>
            <Banner kind="bad" onDismiss={() => setOpenError(null)}>
              {openError()} The file may have been moved or lacks a default app.
              {/* Only point at the "Folder" button when it's actually on screen
                  (it's hidden while a run is in progress). */}
              <Show when={!rs().running && rs().folder}> Use “Folder” below to locate it.</Show>
            </Banner>
          </div>
        </Show>

        {/* NO RESULTS — a finished run that found nothing (the run card is
            hidden because nothing streamed in). */}
        <Show when={noResults()}>
          <div style={{ "margin-top": "22px" }}>
            <Banner kind="warn">
              <strong>No documents found.</strong> Try broader or different terms, enable more
              sources, or check your connection. A source may have been rate-limited — expand any
              issues below for details.
            </Banner>
          </div>
        </Show>

        {/* CANCELLED EMPTY — user Stopped before anything was saved. */}
        <Show when={cancelledEmpty()}>
          <div style={{ "margin-top": "22px" }}>
            <Banner kind="warn">
              <strong>Search cancelled</strong> before any documents were saved.
            </Banner>
          </div>
        </Show>

        {/* ALL OFF-TOPIC — found candidates but the ranker kept none. */}
        <Show when={allOffTopic()}>
          <div style={{ "margin-top": "22px" }}>
            <Banner kind="warn">
              <strong>
                Found {rs().found} result{rs().found === 1 ? "" : "s"}, but none looked relevant
                enough to keep.
              </strong>{" "}
              They were filtered out as off-topic. Try simpler or broader keywords, or lower the
              Search quality (Settings → Search quality → Fast) so fewer results get filtered.
            </Banner>
          </div>
        </Show>

        {/* ISSUES — rendered at canvas level (not inside the run card) so they
            stay visible even when the run card is hidden, e.g. an all-sources-
            failed run has found===0 and no run card, yet the user must see why.
            This is also what the no-results banner's "expand issues below" points
            at. */}
        <Show when={issues().length > 0}>
          <div style={{ "margin-top": "16px" }}>
            <details class="df-issues">
              <summary>{issues().length === 1 ? "1 issue" : `${issues().length} issues`}</summary>
              <ul>
                <For each={issues()}>
                  {(iss) => (
                    <li
                      style={{
                        "--src-color": iss.source ? sourceColor(iss.source) : "var(--ink-3)",
                      }}
                    >
                      <code>{iss.label}</code>
                      <Show when={iss.tag}>
                        <span
                          style={{
                            "font-family": "var(--font-mono)",
                            "font-size": "9px",
                            color: "var(--ink-3)",
                          }}
                        >
                          {iss.tag}
                        </span>
                      </Show>
                      <span style={{ color: "var(--ink-3)" }}>{iss.text}</span>
                    </li>
                  )}
                </For>
              </ul>
            </details>
          </div>
        </Show>

        {/* RUN CARD */}
        <Show when={hasRun()}>
          <div class="df-run-card fade-in" style={{ "margin-top": "22px" }}>
            <div class="df-run-head">
              <div class="df-run-title-row">
                <div class="df-run-title">
                  <Show when={rs().running}>
                    <span class="df-pulse" />
                  </Show>
                  <span class="df-run-q">
                    <span class="q-tag">Run</span>
                    <span class="q-val">&ldquo;{rs().query}&rdquo;</span>
                  </span>
                </div>
                <div style={{ display: "flex", gap: "6px", "align-items": "center" }}>
                  <Show when={rs().running}>
                    <ModelStatusBadge />
                    <span class="df-section-meta" style={{ "font-family": "var(--font-mono)" }}>
                      {rs().active} in flight
                    </span>
                  </Show>
                  <Show when={!rs().running && rs().folder}>
                    <Show when={rs().failed > 0}>
                      <button
                        class="df-btn sm"
                        onClick={() => {
                          // Clear stale export banners so they don't float over
                          // the retry run (handleSearch does the same for a search).
                          setExportedTo(null);
                          setExportError(null);
                          void runStore.retryFailed();
                        }}
                        title="Re-attempt the downloads that failed, into the same library"
                      >
                        <RotateCw size={12} /> Retry {rs().failed} failed
                      </button>
                    </Show>
                    <button
                      class="df-btn sm"
                      onClick={() => void handleExport()}
                      disabled={exporting()}
                    >
                      <Show when={exporting()} fallback={<Archive size={12} />}>
                        <Loader2 size={12} class="spin" />
                      </Show>
                      Export ZIP
                    </button>
                    <button class="df-btn sm" onClick={() => void handleOpenLibrary()}>
                      <BookOpen size={12} /> Library
                    </button>
                    <button
                      class="df-btn sm ghost"
                      onClick={() => {
                        const f = rs().folder;
                        if (f)
                          api
                            .revealInFinder(f)
                            .catch((e) =>
                              uiStore.announce(`Couldn't open this run's folder: ${String(e)}`),
                            );
                      }}
                    >
                      <FolderOpen size={12} /> Folder
                    </button>
                  </Show>
                </div>
              </div>

              <div class="df-stats">
                <div class="df-stat">
                  <span class="df-stat-num">{rs().found}</span>
                  <span class="df-stat-label">found</span>
                </div>
                <div class="df-stat">
                  <span class="df-stat-num ok">{rs().done}</span>
                  <span class="df-stat-label">saved</span>
                </div>
                <Show when={rs().failed > 0}>
                  <div class="df-stat">
                    <span class="df-stat-num bad">{rs().failed}</span>
                    <span class="df-stat-label">failed</span>
                  </div>
                </Show>
                <Show when={rs().rankingRejected > 0}>
                  <div class="df-stat">
                    <span class="df-stat-num" style={{ color: "var(--ink-3)" }}>
                      {rs().rankingRejected}
                    </span>
                    <span class="df-stat-label">off-topic</span>
                  </div>
                </Show>
                <Show when={noTextCount() > 0}>
                  <div
                    class="df-stat"
                    title="Saved, but no machine-readable text could be extracted (e.g. scanned PDFs)"
                  >
                    <span class="df-stat-num" style={{ color: "var(--ink-3)" }}>
                      {noTextCount()}
                    </span>
                    <span class="df-stat-label">no text</span>
                  </div>
                </Show>
                <span style={{ flex: 1 }} />
                <div
                  style={{
                    display: "flex",
                    gap: "10px",
                    "font-family": "var(--font-mono)",
                    "font-size": "11px",
                    color: "var(--ink-3)",
                  }}
                >
                  <For each={Object.entries(fileTypeCounts()).filter(([, v]) => v > 0)}>
                    {([k, v]) => (
                      <span>
                        <span class={`df-ftype ${k}`}>{k}</span>{" "}
                        <span style={{ color: "var(--ink)" }}>{v}</span>
                      </span>
                    )}
                  </For>
                </div>
                <div class="df-stat-label" style={{ "align-self": "center" }}>
                  <Show
                    when={phaseLabel()}
                    fallback={
                      // A finished run shows a word, not "0% complete" (which read
                      // as stuck on the all-off-topic / cancelled cards).
                      <>
                        {rs().running
                          ? `${runStore.overallPct}% complete`
                          : rs().cancelled
                            ? "Cancelled"
                            : "Done"}
                      </>
                    }
                  >
                    <span style={{ color: "var(--accent-ink)", "font-weight": 600 }}>
                      {phaseLabel()}
                    </span>
                  </Show>
                </div>
              </div>

              <div class="df-progress-track">
                <div
                  classList={{ "df-progress-fill": true, shimmer: rs().running }}
                  style={{ width: `${barPct()}%` }}
                />
              </div>

              {/* Pipeline rail */}
              <div style={{ display: "flex", "flex-wrap": "wrap", gap: "10px" }}>
                <For each={pipelineStore.ordered}>
                  {(st) => {
                    const e = () => pipelineStore.stages[st];
                    const tone = () => {
                      const s = e().state;
                      if (s === "done") return "var(--ok)";
                      if (s === "started" || s === "progress") return "var(--accent)";
                      return "var(--ink-4)";
                    };
                    return (
                      <Show when={e().state !== "idle"}>
                        <span
                          style={{
                            display: "flex",
                            "align-items": "center",
                            gap: "4px",
                            "font-family": "var(--font-mono)",
                            "font-size": "9.5px",
                            "text-transform": "uppercase",
                            "letter-spacing": "0.04em",
                            color: "var(--ink-3)",
                          }}
                        >
                          <span
                            style={{
                              width: "6px",
                              height: "6px",
                              "border-radius": "50%",
                              background: tone(),
                            }}
                          />
                          {STAGE_LABEL[st] ?? st}
                          <Show when={e().total}>
                            {" "}
                            {e().count ?? 0}/{e().total}
                          </Show>
                        </span>
                      </Show>
                    );
                  }}
                </For>
              </div>
            </div>

            {/* SPEED STRIP — only while a run is live. Previously also shown when
                `currentMbps() > 0`, which left a frozen throughput number, stale
                avg, and a static sparkline visible after the run ended (the
                sampler doesn't zero speedHist on stop). */}
            <Show when={rs().running}>
              <div class="df-speed">
                <div class="df-speed-item">
                  <span class="df-speed-num">
                    {currentMbps().toFixed(2)}
                    <span
                      style={{ "font-size": "11px", color: "var(--ink-3)", "margin-left": "4px" }}
                    >
                      MB/s
                    </span>
                  </span>
                  <span class="df-speed-label">throughput</span>
                </div>
                <div class="df-speed-spark">
                  <Sparkline values={speedHist()} color="var(--accent)" />
                </div>
                <div class="df-speed-item" style={{ "align-items": "flex-end" }}>
                  <span class="df-speed-num">{avgMbps().toFixed(2)}</span>
                  <span class="df-speed-label">avg {SPEED_WINDOW_SEC}s</span>
                </div>
                <div class="df-speed-item" style={{ "align-items": "flex-end" }}>
                  <span class="df-speed-num">{fmtEta(etaSec())}</span>
                  <span class="df-speed-label">ETA</span>
                </div>
              </div>
            </Show>
            {/* Make a flat 0 MB/s legible during the fetch phase: the graph isn't
                broken, there just aren't any completed downloads yet. */}
            <Show when={rs().running && downloadsStarted() && rs().done === 0}>
              <p
                style={{
                  "font-size": "11px",
                  color: "var(--ink-3)",
                  margin: "4px 0 0",
                  "text-align": "center",
                }}
              >
                No completed downloads yet — resolving links to PDFs and fetching files…
              </p>
            </Show>

            {/* TELEMETRY */}
            <Show when={rs().subQueries.length > 0 || laneSources().length > 0}>
              <div class="df-tel">
                {/* Retry-failed re-runs without a discovery phase, so subQueries
                    stays empty while laneSources() is non-empty — guard this block
                    independently so a retry never renders a dead "Sub-queries (0)". */}
                <Show when={rs().subQueries.length > 0}>
                  <div class="df-tel-block">
                    <div class="df-tel-label">Sub-queries ({rs().subQueries.length})</div>
                    <For each={rs().subQueries}>
                      {(sq, i) => (
                        <div class="df-subq-row">
                          <span class="df-subq-row-num">{String(i() + 1).padStart(2, "0")}</span>
                          <span class="df-subq-row-text">
                            <span>{sq}</span>
                            <span
                              class="df-subq-row-dots"
                              style={{ "--src-color": "var(--accent)" }}
                            >
                              <i class={rs().running ? "live" : "done"} />
                            </span>
                          </span>
                          <span class="df-subq-row-count" />
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <div class="df-tel-block">
                  <div class="df-tel-label">By source</div>
                  <div class="df-lanes">
                    <For each={laneSources()}>
                      {(s) => {
                        const v = () => lanes()[s];
                        // Solid bar = found during discovery, saved during/after
                        // download; the in-flight overlay only shows downloads.
                        const primary = () => (showSaved() ? v().done : v().found);
                        return (
                          <div
                            class="df-lane"
                            style={{ "--src-color": sourceColor(s) }}
                            title={`${SOURCE_LABELS[s]}: ${v().found} found · ${v().done} saved · ${v().inflight} in flight`}
                          >
                            <Show when={downloadsStarted() && v().inflight > 0}>
                              <div
                                class="df-lane-bar inflight"
                                style={{
                                  height: `${Math.min(100, (v().inflight / laneMax()) * 100)}%`,
                                  "margin-bottom": "1px",
                                }}
                              />
                            </Show>
                            <div
                              class="df-lane-bar"
                              style={{ height: `${Math.min(100, (primary() / laneMax()) * 100)}%` }}
                            />
                          </div>
                        );
                      }}
                    </For>
                  </div>
                  <div style={{ display: "flex", gap: "4px" }}>
                    <For each={laneSources()}>
                      {(s) => (
                        <div style={{ flex: 1, "min-width": 0 }}>
                          <div class="df-lane-key">{SOURCE_LABELS[s].slice(0, 4)}</div>
                          <div class="df-lane-val">
                            {showSaved() ? lanes()[s].done + lanes()[s].inflight : lanes()[s].found}
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              </div>
            </Show>

            {/* STREAM */}
            <Show
              when={streamLayout() === "split"}
              fallback={
                <div class="df-stream">
                  <Show when={inFlightDocs().length > 0}>
                    <div class="df-stream-section">
                      <div class="df-stream-label">
                        Downloading{" "}
                        <span style={{ color: "var(--ink-2)", "font-weight": 700 }}>
                          {inFlightDocs().length}
                        </span>
                      </div>
                      {renderInFlightRows()}
                    </div>
                  </Show>
                  <div class="df-stream-section">
                    <div class="df-stream-label">
                      Saved{" "}
                      <span style={{ color: "var(--ok-ink)", "font-weight": 700 }}>
                        {rs().done}
                      </span>
                      <Show when={savedDocs().length > 0}>
                        <span
                          style={{
                            color: "var(--ink-3)",
                            "font-weight": 400,
                            "font-size": "10.5px",
                            "margin-left": "6px",
                          }}
                        >
                          · click to open
                        </span>
                      </Show>
                    </div>
                    <Show
                      when={savedDocs().length > 0}
                      fallback={
                        <div
                          style={{ padding: "12px 0", "font-size": "12px", color: "var(--ink-3)" }}
                        >
                          {rs().running ? "Nothing saved yet…" : "No documents were saved."}
                        </div>
                      }
                    >
                      <Index each={savedDocs()}>
                        {(d) => <DocRow doc={d()} kind="saved" onOpen={openSaved} />}
                      </Index>
                      {renderSavedOverflow()}
                    </Show>
                  </div>
                </div>
              }
            >
              <div class="df-stream-split">
                <section>
                  <div class="df-stream-label" style={{ "padding-left": "4px" }}>
                    Downloading{" "}
                    <span style={{ color: "var(--ink-2)", "font-weight": 700 }}>
                      {inFlightDocs().length}
                    </span>
                  </div>
                  <Show
                    when={inFlightDocs().length > 0}
                    fallback={
                      <div
                        style={{ padding: "12px 0", "font-size": "12px", color: "var(--ink-3)" }}
                      >
                        {!rs().running
                          ? rs().done === 0 && rs().failed === 0
                            ? "None matched closely enough to keep — try broader terms or Fast quality."
                            : "Downloads complete."
                          : downloadsStarted()
                            ? "Queue clearing…"
                            : "Waiting for downloads to start…"}
                      </div>
                    }
                  >
                    {renderInFlightRows()}
                  </Show>
                </section>
                <section
                  style={{ "border-left": "0.5px solid var(--line)", "padding-left": "16px" }}
                >
                  <div class="df-stream-label" style={{ "padding-left": "4px" }}>
                    Saved{" "}
                    <span style={{ color: "var(--ok-ink)", "font-weight": 700 }}>{rs().done}</span>
                  </div>
                  <Show
                    when={savedDocs().length > 0}
                    fallback={
                      <div
                        style={{ padding: "12px 0", "font-size": "12px", color: "var(--ink-3)" }}
                      >
                        {rs().running ? "Nothing saved yet…" : "No documents were saved."}
                      </div>
                    }
                  >
                    <div style={{ "max-height": "360px", "overflow-y": "auto" }}>
                      <Index each={savedDocs()}>
                        {(d) => <DocRow doc={d()} kind="saved" onOpen={openSaved} />}
                      </Index>
                    </div>
                    {renderSavedOverflow()}
                  </Show>
                </section>
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
