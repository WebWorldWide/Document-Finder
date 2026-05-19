import { For, Show, createMemo } from "solid-js";
import { Archive, FolderOpen, BookOpen } from "lucide-solid";
import { runStore } from "@/stores/run";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { api } from "@/lib/tauri";
import { log } from "@/lib/log";
import { formatBytes, formatEta, SOURCE_LABELS } from "@/lib/utils";
import Sparkline from "./Sparkline";

/// Run telemetry card. Shows:
///   1. Title row — pulse dot (when running) + "Run «query»" + right-side
///      actions (Export ZIP / Show folder / Open library when finished;
///      in-flight count when running)
///   2. Stats row — found · saved · failed · off-topic · file-type
///      breakdown · pct complete
///   3. Progress bar (shimmer)
///   4. Speed strip — current MB/s + sparkline + last-20s avg + ETA
///   5. Telemetry — sub-query list (with dots) + per-source lane chart
///   6. Recent completions stream
///
/// Reads everything from runStore — no props.
export default function RunCard() {
  const rs = () => runStore.state;
  const running = () => rs().running;
  const inFlightCount = () => Object.keys(rs().inFlight).length;
  const total = () => rs().total;
  const done = () => rs().done;
  const failed = () => rs().failed;
  const pct = () => runStore.overallPct;

  // File-type counts derived from completed items.
  const fileTypeCounts = createMemo(() => {
    const counts: Record<string, number> = { pdf: 0, epub: 0, html: 0, txt: 0 };
    for (const c of rs().completed) {
      if (c.status !== "done" || !c.local_path) continue;
      const ext = c.local_path.split(".").pop()?.toLowerCase() ?? "";
      if (ext in counts) counts[ext]++;
    }
    return counts;
  });

  // Per-source totals for the lane chart.
  const perSource = createMemo(() => {
    const totals: Record<string, { done: number; inflight: number }> = {};
    for (const id of settings.selectedSources) totals[id] = { done: 0, inflight: 0 };
    for (const c of rs().completed) {
      const key = c.source.startsWith("meta_search/")
        ? c.source.slice("meta_search/".length)
        : c.source;
      if (!totals[key]) totals[key] = { done: 0, inflight: 0 };
      if (c.status === "done") totals[key].done++;
    }
    for (const id of Object.keys(rs().inFlight)) {
      const f = rs().inFlight[id];
      const key = f.source.startsWith("meta_search/")
        ? f.source.slice("meta_search/".length)
        : f.source;
      if (!totals[key]) totals[key] = { done: 0, inflight: 0 };
      totals[key].inflight++;
    }
    return totals;
  });
  const laneMax = createMemo(() => {
    const ps = perSource();
    const ids = Object.keys(ps);
    if (!ids.length) return 1;
    return Math.max(1, ...ids.map((k) => ps[k].done + ps[k].inflight));
  });

  // Sub-query progress: we don't have per-subquery telemetry from Rust, so
  // each sub-query shares the overall progress percentage. The 5-dot
  // visual still tells the user "search is moving."
  function dotState(i: number) {
    const filled = Math.round(pct() / 20);
    if (i < filled) return "done";
    if (i === filled && running()) return "live";
    return "";
  }

  const exporting = () => false; // simple, no async-state knob needed here

  async function handleExport() {
    try {
      const result = await runStore.exportZip();
      if (result) log.info("run", `exported ${result.files} files to ${result.dest}`);
    } catch (e) {
      log.error("run", "export ZIP failed", e);
    }
  }
  async function handleOpenLibrary() {
    if (!rs().folder) return;
    try {
      const info = await api.openLibrary(rs().folder!);
      uiStore.setActiveLibrary(info);
      uiStore.setView("library");
      log.info("library", `opened ${info.name}`);
    } catch (e) {
      log.error("library", "openLibrary failed", e);
    }
  }

  return (
    <div class="df-run-card fade-in">
      <div class="df-run-head">
        <div class="df-run-title-row">
          <div class="df-run-title">
            <Show when={running()}>
              <span class="df-pulse" />
            </Show>
            <span class="df-run-q">
              <span class="q-tag">Run</span>
              <span class="q-val">&ldquo;{rs().query}&rdquo;</span>
            </span>
          </div>
          <div style={{ display: "flex", gap: "6px", "align-items": "center" }}>
            <Show
              when={!running()}
              fallback={
                <span class="df-section-meta" style={{ "font-family": "var(--font-mono)" }}>
                  {inFlightCount()} in flight
                </span>
              }
            >
              <Show when={rs().folder}>
                <button class="df-btn sm" onClick={handleExport} disabled={exporting()}>
                  <Archive size={12} /> Export ZIP
                </button>
                <button class="df-btn sm" onClick={() => api.revealInFinder(rs().folder!)}>
                  <FolderOpen size={12} /> Show folder
                </button>
                <button class="df-btn sm" onClick={handleOpenLibrary}>
                  <BookOpen size={12} /> Open library
                </button>
              </Show>
            </Show>
          </div>
        </div>

        <div class="df-stats">
          <div class="df-stat">
            <span class="df-stat-num">{rs().found}</span>
            <span class="df-stat-label">found</span>
          </div>
          <div class="df-stat">
            <span class="df-stat-num ok">{done()}</span>
            <span class="df-stat-label">saved</span>
          </div>
          <Show when={failed() > 0}>
            <div class="df-stat">
              <span class="df-stat-num bad">{failed()}</span>
              <span class="df-stat-label">failed</span>
            </div>
          </Show>
          <Show when={rs().filteredCount > 0}>
            <div class="df-stat">
              <span class="df-stat-num" style={{ color: "var(--ink-3)" }}>
                {rs().filteredCount}
              </span>
              <span class="df-stat-label">off-topic</span>
            </div>
          </Show>
          <div style={{ flex: 1 }} />
          <div style={{
            display: "flex", gap: "10px",
            "font-family": "var(--font-mono)",
            "font-size": "11px", color: "var(--ink-3)",
          }}>
            <For each={Object.entries(fileTypeCounts())}>{([k, v]) => (
              <Show when={v > 0}>
                <span>
                  <span class={`df-ftype ${k}`}>{k}</span>{" "}
                  <span style={{ color: "var(--ink)" }}>{v}</span>
                </span>
              </Show>
            )}</For>
          </div>
          <div class="df-stat-label" style={{ "align-self": "center" }}>
            {pct()}% complete
          </div>
        </div>

        <div class="df-progress-track">
          <div class="df-progress-fill" style={{ width: `${pct()}%` }} />
        </div>
      </div>

      {/* Speed strip */}
      <Show when={running() || rs().speedHist.length > 0}>
        <div class="df-speed">
          <div class="df-speed-item">
            <span class="df-speed-num">
              {runStore.currentMbps.toFixed(2)}
              <span style={{ "font-size": "11px", color: "var(--ink-3)", "margin-left": "4px" }}>
                MB/s
              </span>
            </span>
            <span class="df-speed-label">throughput</span>
          </div>
          <div class="df-speed-spark">
            <Sparkline values={rs().speedHist} />
          </div>
          <div class="df-speed-item" style={{ "align-items": "flex-end" }}>
            <span class="df-speed-num">
              {runStore.avgMbps.toFixed(2)}
              <span style={{ "font-size": "11px", color: "var(--ink-3)", "margin-left": "4px" }}>
                avg
              </span>
            </span>
            <span class="df-speed-label">last 20s</span>
          </div>
          <div class="df-speed-item" style={{ "align-items": "flex-end" }}>
            <span class="df-speed-num">{formatEta(runStore.etaSec)}</span>
            <span class="df-speed-label">ETA</span>
          </div>
        </div>
      </Show>

      {/* Telemetry */}
      <Show when={rs().subQueries.length > 0 || settings.selectedSources.length > 0}>
        <div class="df-tel">
          <div class="df-tel-block">
            <div class="df-tel-label">
              Sub-queries ({rs().subQueries.length})
            </div>
            <For each={rs().subQueries}>{(sq, i) => (
              <div class="df-subq-row">
                <span class="df-subq-row-num">
                  {String(i() + 1).padStart(2, "0")}
                </span>
                <span class="df-subq-row-text">
                  <span>{sq.text}</span>
                  <span class="df-subq-row-dots">
                    <For each={[0, 1, 2, 3, 4]}>{(k) => (
                      <i class={dotState(k)} />
                    )}</For>
                  </span>
                </span>
                <span class="df-subq-row-count">
                  {sq.done > 0 || sq.found > 0 ? `${sq.done}/${sq.found}` : ""}
                </span>
              </div>
            )}</For>
          </div>
          <div class="df-tel-block">
            <div class="df-tel-label">By source</div>
            <div class="df-lanes">
              <For each={settings.selectedSources}>{(id) => {
                const v = () => perSource()[id] ?? { done: 0, inflight: 0 };
                const max = laneMax();
                return (
                  <div
                    class="df-lane"
                    style={{ "--src-color": `var(--src-${id})` } as Record<string, string>}
                    title={`${SOURCE_LABELS[id] ?? id}: ${v().done} saved · ${v().inflight} in flight`}
                  >
                    <Show when={v().inflight > 0}>
                      <div
                        class="df-lane-bar inflight"
                        style={{
                          height: `${(v().inflight / max) * 100}%`,
                          "margin-bottom": "1px",
                        }}
                      />
                    </Show>
                    <div
                      class="df-lane-bar"
                      style={{ height: `${(v().done / max) * 100}%` }}
                    />
                  </div>
                );
              }}</For>
            </div>
          </div>
        </div>
      </Show>

      {/* Recent completions — layout flips between stacked (default) and a
          two-column split based on settings.streamLayout. */}
      <Show when={rs().completed.length > 0 || Object.keys(rs().inFlight).length > 0}>
        <div
          class="df-stream"
          classList={{ "df-stream-split": settings.streamLayout === "split" }}
        >
          <Show when={Object.keys(rs().inFlight).length > 0}>
            <div class="df-stream-section">
              <div class="df-stream-label">
                <span>Downloading</span>
                <span style={{ color: "var(--ink-3)", "font-weight": "400" }}>
                  ({Object.keys(rs().inFlight).length})
                </span>
              </div>
              <For each={Object.values(rs().inFlight)}>{(item) => {
                const pct = () => item.total > 0
                  ? Math.round((item.downloaded / item.total) * 100)
                  : 0;
                return (
                  <div
                    class="df-doc in-flight"
                    style={{ "--src-color": `var(--src-${item.source.replace(/-/g, "_").replace("meta_search/", "")})` } as Record<string, string>}
                  >
                    <span class="df-doc-source">{SOURCE_LABELS[item.source] ?? item.source}</span>
                    <div class="df-doc-main">
                      <div class="df-doc-title">{item.title}</div>
                      <div class="df-doc-progress">
                        <div class="df-progress-track">
                          <div class="df-progress-fill" style={{ width: `${pct()}%` }} />
                        </div>
                        <span class="df-doc-bytes">
                          {item.total > 0
                            ? `${formatBytes(item.downloaded)} / ${formatBytes(item.total)}`
                            : formatBytes(item.downloaded)}
                        </span>
                      </div>
                    </div>
                  </div>
                );
              }}</For>
            </div>
          </Show>

          <Show when={rs().completed.length > 0}>
            <div class="df-stream-section">
              <div class="df-stream-label">
                <span>Saved</span>
                <span style={{ color: "var(--ink-3)", "font-weight": "400" }}>
                  ({rs().completed.length})
                </span>
              </div>
              <For each={rs().completed.slice(-30).reverse()}>{(item) => (
                <div
                  class="df-doc"
                  style={{ "--src-color": `var(--src-${item.source.replace(/-/g, "_").replace("meta_search/", "")})` } as Record<string, string>}
                >
                  <span class="df-doc-source">{SOURCE_LABELS[item.source] ?? item.source}</span>
                  <div class="df-doc-main">
                    <div class="df-doc-title">{item.title}</div>
                  </div>
                  <span
                    class="df-doc-status"
                    classList={{ ok: item.status === "done", bad: item.status === "failed" }}
                  >
                    <Show when={item.status === "done"} fallback={item.error ?? "failed"}>
                      saved
                    </Show>
                  </span>
                </div>
              )}</For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
