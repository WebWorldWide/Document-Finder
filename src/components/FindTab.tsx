import { createSignal, createMemo, createEffect, onCleanup, untrack, Show, For } from "solid-js";
import { Search, Square, Sparkles, Archive, FolderOpen, BookOpen, Loader2 } from "lucide-solid";
import DocRow, { type StreamDoc } from "./DocRow";
import SourcePanel from "./SourcePanel";
import Sparkline from "./Sparkline";
import Banner from "./Banner";
import ModelStatusBadge from "./ModelStatusBadge";
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
  formatBytes,
  ftypeFromPath,
  type SourceId,
} from "@/lib/utils";
import { humanizeDownloadError, humanizeSourceKind, issueKindTag } from "@/lib/errors";

const PRIMARY_SOURCES: SourceId[] = ALL_SOURCES.filter((s) => !META_SEARCH_COVERED.includes(s));

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
  return source;
}

export default function FindTab() {
  const [query, setQuery] = createSignal("");
  const [exporting, setExporting] = createSignal(false);
  const [exportedTo, setExportedTo] = createSignal<string | null>(null);
  const [exportError, setExportError] = createSignal<string | null>(null);
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

  const hasRun = createMemo(
    () => rs().running || rs().completed.length > 0 || Object.keys(rs().inFlight).length > 0,
  );

  // ---- Live throughput sampler — samples cumulative bytes every 600ms while a
  // run is active and pushes MB/s into a rolling window for the sparkline.
  createEffect(() => {
    if (!rs().running) return;
    let lastBytes = untrack(() => runStore.state.bytesDownloaded);
    let lastT = performance.now();
    setSpeedHist(new Array(32).fill(0));
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
    }, 600);
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
  const etaSec = createMemo(() => {
    const remaining = Math.max(0, rs().total - rs().done - rs().failed);
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

  const downloadsStarted = () => {
    const st = pipelineStore.stages.download.state;
    return st === "started" || st === "progress" || st === "done";
  };

  // Bar tracks download completion once downloads begin; before that it tracks
  // the active stage's progress (e.g. ranking 181/500) so it visibly advances.
  const barPct = createMemo(() => {
    if (downloadsStarted()) return runStore.overallPct;
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

  const lanes = createMemo(() => {
    const totals: Record<string, { done: number; inflight: number }> = {};
    for (const id of PRIMARY_SOURCES) totals[id] = { done: 0, inflight: 0 };
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
  const laneSources = () => PRIMARY_SOURCES.filter((s) => settings.selectedSources.includes(s));
  const laneMax = () =>
    Math.max(1, ...laneSources().map((s) => lanes()[s].done + lanes()[s].inflight));

  const inFlightDocs = (): StreamDoc[] =>
    Object.values(rs().inFlight).map((d) => ({
      source: d.source,
      title: d.title,
      downloaded: d.downloaded,
      total: d.total,
      ftype: ftypeFromPath(d.url),
    }));
  const savedDocs = (): StreamDoc[] =>
    rs()
      .completed.filter((c) => c.status === "done")
      .slice(-40)
      .reverse()
      .map((c) => ({
        source: c.source,
        title: c.title,
        status: "done" as const,
        ftype: ftypeFromPath(c.local_path),
        // Carry the on-disk size so DocRow can render it next to the checkmark.
        bytes: c.bytes,
      }));

  const issues = createMemo(() => {
    const out: { source?: string; label: string; tag?: string; text: string }[] = [];
    for (const i of rs().sourceIssues) {
      out.push({
        source: i.source,
        label: SOURCE_LABELS[i.source] ?? i.source,
        tag: issueKindTag(i.kind) + (i.count > 1 ? ` ×${i.count}` : ""),
        text: humanizeSourceKind(i.kind, i.source),
      });
    }
    for (const f of rs().completed.filter((c) => c.status === "failed")) {
      out.push({
        source: f.source,
        label: SOURCE_LABELS[f.source] ?? f.source,
        text: `${f.title.slice(0, 48)} — ${humanizeDownloadError(f.error)}`,
      });
    }
    return out;
  });

  // ---- source toggles ----
  function enableAll() {
    setSettings("selectedSources", (prev) => [...new Set([...prev, ...PRIMARY_SOURCES])]);
    saveSettings();
  }
  function disableAll() {
    setSettings("selectedSources", (prev) => prev.filter((s) => !PRIMARY_SOURCES.includes(s)));
    saveSettings();
  }
  function invert() {
    setSettings("selectedSources", (prev) => {
      const set = new Set(prev);
      for (const s of PRIMARY_SOURCES) {
        if (set.has(s)) set.delete(s);
        else set.add(s);
      }
      return [...set];
    });
    saveSettings();
  }

  async function handleSearch() {
    if (!query().trim() || rs().running || settings.selectedSources.length === 0) return;
    setExportedTo(null);
    await runStore.startSearch(query());
  }
  async function handleExport() {
    setExporting(true);
    setExportError(null);
    try {
      const result = await runStore.exportZip();
      if (result) setExportedTo(result.dest);
    } catch (e) {
      // Was silently swallowed with a misleading "surfaced elsewhere" comment;
      // export errors have no other surface, so show them here.
      setExportError(humanizeDownloadError(String(e)));
    } finally {
      setExporting(false);
    }
  }
  async function handleOpenLibrary() {
    const folder = rs().folder;
    if (!folder) return;
    try {
      const info = await api.openLibrary(folder);
      uiStore.setActiveLibrary(info);
      uiStore.setView("library");
    } catch {
      /* ignore */
    }
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
              <span class="df-kbd">Ctrl</span>
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
                onClick={() => {
                  setStopping(true);
                  void api.cancelRun();
                }}
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
                      class="df-btn sm"
                      onClick={() => toggleSource(src)}
                      aria-pressed={on()}
                      style={
                        on()
                          ? {
                              "border-color": sourceColor(src),
                              color: sourceColor(src),
                              background:
                                "color-mix(in oklch, " + sourceColor(src) + " 8%, transparent)",
                            }
                          : {}
                      }
                    >
                      {SOURCE_LABELS[src]}
                    </button>
                  );
                }}
              </For>
            </div>
          </details>

          <Show when={settings.selectedSources.length === 0}>
            <p style={{ "margin-top": "10px", "font-size": "12px", color: "var(--bad)" }}>
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
                        if (f) api.revealInFinder(f);
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
                  <Show when={phaseLabel()} fallback={<>{runStore.overallPct}% complete</>}>
                    <span style={{ color: "var(--accent)", "font-weight": 600 }}>
                      {phaseLabel()}
                    </span>
                  </Show>
                </div>
              </div>

              <div class="df-progress-track">
                <div class="df-progress-fill shimmer" style={{ width: `${barPct()}%` }} />
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

            {/* SPEED STRIP */}
            <Show when={rs().running || currentMbps() > 0}>
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
                  <span class="df-speed-label">avg 19s</span>
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
                <div class="df-tel-block">
                  <div class="df-tel-label">Sub-queries ({rs().subQueries.length})</div>
                  <For each={rs().subQueries}>
                    {(sq, i) => (
                      <div class="df-subq-row">
                        <span class="df-subq-row-num">{String(i() + 1).padStart(2, "0")}</span>
                        <span class="df-subq-row-text">
                          <span>{sq}</span>
                          <span class="df-subq-row-dots" style={{ "--src-color": "var(--accent)" }}>
                            <i class={rs().running ? "live" : "done"} />
                          </span>
                        </span>
                        <span class="df-subq-row-count" />
                      </div>
                    )}
                  </For>
                </div>
                <div class="df-tel-block">
                  <div class="df-tel-label">By source</div>
                  <div class="df-lanes">
                    <For each={laneSources()}>
                      {(s) => {
                        const v = () => lanes()[s];
                        return (
                          <div
                            class="df-lane"
                            style={{ "--src-color": sourceColor(s) }}
                            title={`${SOURCE_LABELS[s]}: ${v().done} saved · ${v().inflight} in flight`}
                          >
                            <Show when={v().inflight > 0}>
                              <div
                                class="df-lane-bar inflight"
                                style={{
                                  height: `${(v().inflight / laneMax()) * 100}%`,
                                  "margin-bottom": "1px",
                                }}
                              />
                            </Show>
                            <div
                              class="df-lane-bar"
                              style={{ height: `${(v().done / laneMax()) * 100}%` }}
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
                          <div class="df-lane-val">{lanes()[s].done + lanes()[s].inflight}</div>
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
                      <For each={inFlightDocs()}>{(d) => <DocRow doc={d} kind="in-flight" />}</For>
                    </div>
                  </Show>
                  <div class="df-stream-section">
                    <div class="df-stream-label">
                      Saved{" "}
                      <span style={{ color: "var(--ok)", "font-weight": 700 }}>{rs().done}</span>
                    </div>
                    <For each={savedDocs()}>{(d) => <DocRow doc={d} kind="saved" />}</For>
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
                        Nothing in flight — queue clearing.
                      </div>
                    }
                  >
                    <For each={inFlightDocs()}>{(d) => <DocRow doc={d} kind="in-flight" />}</For>
                  </Show>
                </section>
                <section
                  style={{ "border-left": "0.5px solid var(--line)", "padding-left": "16px" }}
                >
                  <div class="df-stream-label" style={{ "padding-left": "4px" }}>
                    Saved{" "}
                    <span style={{ color: "var(--ok)", "font-weight": 700 }}>{rs().done}</span>
                  </div>
                  <div style={{ "max-height": "360px", "overflow-y": "auto" }}>
                    <For each={savedDocs()}>{(d) => <DocRow doc={d} kind="saved" />}</For>
                  </div>
                </section>
              </div>
            </Show>

            {/* ISSUES */}
            <Show when={issues().length > 0}>
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
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
