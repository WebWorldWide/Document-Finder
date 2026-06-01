import { createSignal, onMount, Show, Switch, Match, For } from "solid-js";
import {
  FolderOpen,
  FileText,
  Download,
  Loader2,
  CheckCircle2,
  Sparkles,
  RefreshCw,
  AlertCircle,
} from "lucide-solid";
import { api, type LogInfo } from "@/lib/tauri";
import { settings, setSettings, saveSettings, type Quality } from "@/stores/settings";
import { modelsStore } from "@/stores/models";
import ModelDownloadCard from "./ModelDownloadCard";
import MetaSearchHealthBar from "./MetaSearchHealthBar";
import ThemePicker from "./ThemePicker";
import Logo from "./Logo";
import { formatBytes } from "@/lib/utils";

export default function SettingsView() {
  const [logInfo, setLogInfo] = createSignal<LogInfo | null>(null);
  const [warming, setWarming] = createSignal(false);

  async function handleWarmEmbedding() {
    if (warming()) return;
    setWarming(true);
    try {
      await modelsStore.warmEmbedding();
    } catch (e) {
      console.error("warm embedding failed:", e);
      setWarming(false);
      return;
    }
    // warm runs in the background; poll readiness so the row updates live, and
    // clear the spinner once it's loaded/on-disk (or after ~45s if it fails).
    let tries = 0;
    const poll = setInterval(() => {
      tries += 1;
      void modelsStore.refresh();
      if (modelsStore.embeddingState !== "absent" || tries >= 30) {
        clearInterval(poll);
        setWarming(false);
      }
    }, 1500);
  }

  onMount(async () => {
    setLogInfo(await api.runLogInfo().catch(() => null));
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  function numInput(field: "perSource" | "maxTotal" | "concurrency") {
    return (e: InputEvent & { currentTarget: HTMLInputElement }) => {
      const v = parseInt(e.currentTarget.value, 10);
      if (!isNaN(v) && v > 0) {
        setSettings(field, v);
        saveSettings();
      }
    };
  }

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <div>
          <div class="df-eyebrow">Settings</div>
          <h1 class="df-canvas-title">Settings</h1>
        </div>
      </div>

      <div class="df-canvas-body">
        <div class="df-settings-wrap">
          {/* Themes */}
          <section class="df-section">
            <h2>Themes</h2>
            <p class="hint">
              Pick a base theme, an accent color, the UI density, and how the live download stream
              is laid out.
            </p>
            <ThemePicker />
          </section>

          {/* Library folder */}
          <section class="df-section">
            <h2>Library folder</h2>
            <p class="hint">
              Each search creates its own collection folder here, with a SQLite index and the
              extracted text.
            </p>
            <div class="df-field">
              <label>Folder path</label>
              <input
                class="mono"
                type="text"
                value={settings.libraryRoot}
                onInput={(e) => {
                  setSettings("libraryRoot", e.currentTarget.value);
                  saveSettings();
                }}
              />
            </div>
          </section>

          {/* Discovery limits */}
          <section class="df-section">
            <h2>Discovery limits</h2>
            <p class="hint">
              Tune throughput. Higher numbers find more but take longer and trip more rate limits.
            </p>
            <div class="df-field-row">
              <div class="df-field">
                <label>Per source</label>
                <input
                  type="number"
                  min="1"
                  value={settings.perSource}
                  onInput={numInput("perSource")}
                />
                <span class="help">Docs per source, per sub-query</span>
              </div>
              <div class="df-field">
                <label>Max total</label>
                <input
                  type="number"
                  min="1"
                  value={settings.maxTotal}
                  onInput={numInput("maxTotal")}
                />
                <span class="help">Hard cap across all sources</span>
              </div>
              <div class="df-field">
                <label>Parallel downloads</label>
                <input
                  type="number"
                  min="1"
                  max="32"
                  value={settings.concurrency}
                  onInput={numInput("concurrency")}
                />
                <span class="help">Higher is faster, more rate limits</span>
              </div>
            </div>
          </section>

          {/* Search quality */}
          <section class="df-section">
            <h2>Search quality</h2>
            <p class="hint">
              How hard the app works to rank results. The keyword baseline (TF-IDF · RRF ·
              authority) always runs.
            </p>
            <div
              style={{
                display: "flex",
                gap: "4px",
                padding: "4px",
                background: "var(--card-2)",
                border: "0.5px solid var(--line)",
                "border-radius": "var(--r-2)",
                "margin-bottom": "12px",
              }}
            >
              <QualityTab
                q="fast"
                label="Fast"
                caption="Lexical only · instant"
                active={settings.quality === "fast"}
              />
              <QualityTab
                q="balanced"
                label="Balanced"
                caption={
                  modelsStore.embeddingReady ? "+ semantic rerank · ~5s" : "needs embedding model"
                }
                active={settings.quality === "balanced"}
                disabled={!modelsStore.embeddingReady}
              />
              <QualityTab
                q="thorough"
                label="Thorough"
                caption={modelsStore.llmReady ? "+ LLM expand & filter" : "needs LLM model"}
                active={settings.quality === "thorough"}
                disabled={!modelsStore.llmReady}
              />
            </div>
            <p
              style={{
                "font-size": "11.5px",
                color: "var(--ink-3)",
                "line-height": 1.5,
                margin: "0 0 12px",
              }}
            >
              {settings.quality === "fast" &&
                "Keyword scoring across all sources. Returns immediately. No models needed."}
              {settings.quality === "balanced" &&
                "Adds semantic reranking via the embedding model — the top results are re-scored by query meaning, not just keyword overlap."}
              {settings.quality === "thorough" &&
                "Full AI pipeline: the LLM generates extra sub-queries, then semantic reranking, then an LLM pass that judges borderline results. Several seconds slower."}
            </p>
            <RankingToggle
              checked={settings.useCitationGraph}
              onToggle={(v) => {
                setSettings("useCitationGraph", v);
                saveSettings();
              }}
              label="Citation-graph reasoning"
              detail="Cross-references Semantic Scholar to boost papers cited by other top results. Slow."
            />
          </section>

          {/* AI Models */}
          <section class="df-section">
            <h2 style={{ display: "flex", "align-items": "center", gap: "6px" }}>
              <Sparkles size={14} style={{ color: "var(--accent)" }} /> AI models
            </h2>
            <p class="hint">
              Local models power semantic reranking and LLM query expansion + filtering.
              <Show when={modelsStore.totalDiskBytes > 0}>
                {" "}
                {formatBytes(modelsStore.totalDiskBytes)} on disk.
              </Show>
            </p>

            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "8px",
                padding: "9px 12px",
                background: "var(--card-2)",
                "border-radius": "var(--r-2)",
                "font-size": "11.5px",
                "margin-bottom": "12px",
              }}
            >
              <Switch>
                <Match when={modelsStore.embeddingState === "loaded"}>
                  <CheckCircle2 size={12} style={{ color: "var(--ok)" }} />
                  <span style={{ "font-weight": 500 }}>Embedding model</span>
                  <span style={{ color: "var(--ink-3)" }}>· ready</span>
                </Match>
                <Match when={modelsStore.embeddingState === "downloaded"}>
                  <CheckCircle2 size={12} style={{ color: "var(--ok)" }} />
                  <span style={{ "font-weight": 500 }}>Embedding model</span>
                  <span style={{ color: "var(--ink-3)" }}>
                    · downloaded · loads on first semantic search
                  </span>
                </Match>
                <Match when={modelsStore.embeddingState === "absent"}>
                  <span
                    style={{
                      width: "9px",
                      height: "9px",
                      "border-radius": "50%",
                      border: "1.5px solid var(--ink-4)",
                      "flex-shrink": 0,
                    }}
                  />
                  <span style={{ "font-weight": 500 }}>Embedding model</span>
                  <span style={{ color: "var(--ink-3)" }}>
                    · not downloaded · ~33 MB, fetched automatically on first semantic search
                  </span>
                  <span style={{ flex: 1 }} />
                  <button
                    class="df-btn sm"
                    disabled={warming()}
                    onClick={() => void handleWarmEmbedding()}
                  >
                    <Show when={warming()} fallback={<Download size={12} />}>
                      <Loader2 size={12} class="spin" />
                    </Show>
                    {warming() ? "Downloading…" : "Download now"}
                  </button>
                </Match>
              </Switch>
            </div>

            <Show when={modelsStore.state.error}>
              <div
                style={{
                  display: "flex",
                  gap: "8px",
                  padding: "10px 12px",
                  "border-radius": "var(--r-2)",
                  color: "var(--bad)",
                  background: "var(--bad-soft)",
                  "font-size": "12px",
                }}
              >
                <AlertCircle size={14} style={{ "margin-top": "1px", "flex-shrink": 0 }} />
                <div style={{ flex: 1 }}>
                  <p style={{ "font-weight": 600, margin: 0 }}>
                    Couldn&rsquo;t load the model list
                  </p>
                  <p style={{ margin: "2px 0 8px", opacity: 0.9, "word-break": "break-word" }}>
                    {modelsStore.state.error}
                  </p>
                  <button class="df-btn sm" onClick={() => void modelsStore.refresh()}>
                    <RefreshCw size={11} /> Retry
                  </button>
                </div>
              </div>
            </Show>

            <Show
              when={
                !modelsStore.state.error &&
                modelsStore.state.loading &&
                modelsStore.state.models.length === 0
              }
            >
              <div
                style={{
                  display: "flex",
                  gap: "8px",
                  "align-items": "center",
                  color: "var(--ink-3)",
                  "font-size": "11.5px",
                }}
              >
                <Loader2 size={12} class="spin" /> Loading…
              </div>
            </Show>

            <Show when={modelsStore.state.models.length > 0}>
              <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
                <For each={modelsStore.state.models}>
                  {(model) => <ModelDownloadCard model={model} />}
                </For>
              </div>
            </Show>
          </section>

          {/* Web search */}
          <section class="df-section">
            <h2>Web search</h2>
            <p class="hint">Six engines queried in parallel, then merged and deduped — no setup.</p>
            <div
              style={{
                display: "flex",
                "align-items": "flex-start",
                gap: "8px",
                padding: "10px 12px",
                background: "var(--card-2)",
                "border-radius": "var(--r-2)",
                "font-size": "12px",
                "margin-bottom": "12px",
              }}
            >
              <CheckCircle2
                size={14}
                style={{ color: "var(--ok)", "margin-top": "1px", "flex-shrink": 0 }}
              />
              <span>
                <strong>Ready.</strong> DuckDuckGo · Brave · Bing · Mojeek · Marginalia · Startpage,
                with a built-in SearXNG fallback.
              </span>
            </div>
            <MetaSearchHealthBar />
          </section>

          {/* Run log */}
          <section class="df-section">
            <h2>Run log</h2>
            <p class="hint">
              Every query, source error, and download outcome is logged here. Share this file if you
              report a problem.
            </p>
            <Show when={logInfo()} fallback={<p class="hint">Unavailable.</p>}>
              {(info) => (
                <div style={{ display: "flex", "flex-direction": "column", gap: "10px" }}>
                  <code
                    title={info().path}
                    style={{
                      display: "block",
                      overflow: "hidden",
                      "text-overflow": "ellipsis",
                      "white-space": "nowrap",
                      background: "var(--canvas)",
                      border: "0.5px solid var(--line-2)",
                      "border-radius": "var(--r-2)",
                      padding: "9px 12px",
                      "font-family": "var(--font-mono)",
                      "font-size": "11px",
                      color: "var(--ink-2)",
                    }}
                  >
                    {info().path}
                  </code>
                  <p class="hint" style={{ margin: 0 }}>
                    {info().exists ? formatBytes(info().size_bytes) : "Not yet written"}
                  </p>
                  <div style={{ display: "flex", gap: "8px" }}>
                    <button
                      class="df-btn sm"
                      onClick={() => api.revealInFinder(info().path)}
                      disabled={!info().exists}
                    >
                      <FolderOpen size={12} /> Show in folder
                    </button>
                    <button
                      class="df-btn sm ghost"
                      onClick={async () => setLogInfo(await api.runLogInfo().catch(() => null))}
                    >
                      <FileText size={12} /> Refresh
                    </button>
                  </div>
                </div>
              )}
            </Show>
          </section>

          {/* About */}
          <section
            class="df-section"
            style={{ display: "flex", "align-items": "center", gap: "14px" }}
          >
            <Logo size={44} style={{ "border-radius": "10px", "flex-shrink": 0 }} />
            <div style={{ flex: 1 }}>
              <h2 style={{ margin: "0 0 2px" }}>Document Finder</h2>
              <p
                style={{
                  "font-size": "12px",
                  color: "var(--ink-3)",
                  margin: 0,
                  "font-family": "var(--font-mono)",
                }}
              >
                Tauri 2 · Rust · Solid.js · SQLite.
              </p>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function RankingToggle(props: {
  checked: boolean;
  onToggle: (v: boolean) => void;
  label: string;
  detail: string;
  disabled?: boolean;
}) {
  return (
    <label
      style={{ display: "flex", "align-items": "flex-start", gap: "8px", "font-size": "12px" }}
      classList={{ "opacity-50": !!props.disabled }}
    >
      <input
        type="checkbox"
        checked={props.checked && !props.disabled}
        disabled={!!props.disabled}
        onChange={(e) => props.onToggle(e.currentTarget.checked)}
        style={{ "margin-top": "2px" }}
      />
      <span style={{ flex: 1 }}>
        <span style={{ "font-weight": 500 }}>{props.label}</span>{" "}
        <span style={{ "font-size": "11px", color: "var(--ink-3)" }}>· {props.detail}</span>
      </span>
    </label>
  );
}

function QualityTab(props: {
  q: Quality;
  label: string;
  caption: string;
  active: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={() => {
        if (props.disabled) return;
        setSettings("quality", props.q);
        saveSettings();
      }}
      disabled={props.disabled}
      class="pill-toggle"
      classList={{ "is-active": props.active }}
      style={{
        flex: 1,
        padding: "8px 12px",
        "text-align": "center",
        ...(props.disabled ? { opacity: 0.8, cursor: "not-allowed" } : {}),
      }}
    >
      <div style={{ "font-size": "12px", "font-weight": 600, "line-height": 1.2 }}>
        {props.label}
      </div>
      <div
        style={{
          "margin-top": "2px",
          "font-size": "10px",
          "line-height": 1.2,
          color: props.active ? "var(--accent-fg)" : "var(--ink-3)",
          opacity: props.active ? 0.85 : 1,
        }}
      >
        {props.caption}
      </div>
    </button>
  );
}
