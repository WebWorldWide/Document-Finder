import { createSignal, onMount, Show, For } from "solid-js";
import { FolderOpen, FileText, Loader2, CheckCircle2, Sparkles, RefreshCw, AlertCircle } from "lucide-solid";
import { api, type LogInfo } from "@/lib/tauri";
import { settings, setSettings, saveSettings, type Quality } from "@/stores/settings";
import { modelsStore } from "@/stores/models";
import ModelDownloadCard from "./ModelDownloadCard";
import MetaSearchHealthBar from "./MetaSearchHealthBar";
import { formatBytes } from "@/lib/utils";

export default function SettingsView() {
  const [logInfo, setLogInfo] = createSignal<LogInfo | null>(null);

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
    <div class="h-full overflow-y-auto">
      <div class="mx-auto max-w-2xl space-y-6 p-6 pt-10">
        <h1 class="text-xl font-semibold text-embossed">Settings</h1>

        {/* Discovery settings */}
        <section class="material-linen p-5">
          <h2 class="mb-4 text-sm font-semibold text-embossed">Discovery</h2>
          <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
            <label class="block">
              <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">Per source</span>
              <input
                type="number"
                min="1"
                value={settings.perSource}
                onInput={numInput("perSource")}
                class="surface-input w-full px-3 py-2 text-sm outline-none"
              />
              <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">Max docs per source per sub-query</p>
            </label>
            <label class="block">
              <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">Max total</span>
              <input
                type="number"
                min="1"
                value={settings.maxTotal}
                onInput={numInput("maxTotal")}
                class="surface-input w-full px-3 py-2 text-sm outline-none"
              />
              <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">Hard cap across all sources</p>
            </label>
            <label class="block">
              <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">Parallel downloads</span>
              <input
                type="number"
                min="1"
                max="32"
                value={settings.concurrency}
                onInput={numInput("concurrency")}
                class="surface-input w-full px-3 py-2 text-sm outline-none"
              />
              <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">Higher = faster but more rate limits</p>
            </label>
          </div>
        </section>

        {/* AI Models */}
        <section class="material-aluminum p-5">
          <div class="mb-1 flex items-center gap-2">
            <Sparkles size={14} class="text-[var(--color-primary)]" />
            <h2 class="text-sm font-semibold text-embossed">AI Models</h2>
          </div>
          <Show when={modelsStore.totalDiskBytes > 0}>
            <p class="mb-2 text-[10px] text-[var(--color-muted-foreground)]">
              {formatBytes(modelsStore.totalDiskBytes)} on disk
            </p>
          </Show>
          <p class="mb-3 text-xs leading-relaxed text-[var(--color-muted-foreground)]">
            Local AI models power semantic reranking and LLM query expansion +
            borderline filtering. Everything runs offline — no API keys, no
            telemetry. Models can be deleted any time to reclaim disk.
          </p>

          {/* Embedding model is managed by fastembed itself — no explicit
            * download button. Show a passive status row so the user knows
            * what's happening on first semantic search. */}
          <div class="surface-raised-subtle mb-3 flex items-center gap-2 px-3 py-2 text-[11px]">
            <Show
              when={modelsStore.state.embeddingLoaded}
              fallback={
                <>
                  <Loader2 size={12} class="text-[var(--color-foreground-muted)]" />
                  <span class="font-medium">Embedding model</span>
                  <span class="text-[var(--color-foreground-muted)]">
                    · auto-managed · loads on first semantic search
                  </span>
                </>
              }
            >
              <CheckCircle2 size={12} style={{ color: "var(--color-success)" }} />
              <span class="font-medium">Embedding model</span>
              <span class="text-[var(--color-foreground-muted)]">· ready</span>
            </Show>
          </div>
          {/* Three explicit states: error → red banner + retry, loading →
            * spinner, loaded → cards. Previously a single Show with a
            * "Loading…" fallback masked rejected listModels() calls
            * forever. */}
          <Show when={modelsStore.state.error}>
            <div class="surface-raised-sm flex items-start gap-2 p-3 text-xs text-[var(--color-destructive)]">
              <AlertCircle size={14} class="mt-0.5 shrink-0" />
              <div class="flex-1">
                <p class="font-medium">Couldn't load models</p>
                <p class="mt-0.5 break-words opacity-90">
                  {modelsStore.state.error}
                </p>
                <button
                  onClick={() => void modelsStore.refresh()}
                  class="btn-tactile mt-2 flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-medium"
                >
                  <RefreshCw size={11} />
                  Retry
                </button>
              </div>
            </div>
          </Show>

          <Show when={!modelsStore.state.error && modelsStore.state.loading && modelsStore.state.models.length === 0}>
            <div class="flex items-center gap-2 text-[11px] text-[var(--color-muted-foreground)]">
              <Loader2 size={12} class="animate-spin" />
              Loading…
            </div>
          </Show>

          <Show when={!modelsStore.state.error && !modelsStore.state.loading && modelsStore.state.models.length === 0}>
            <p class="text-[11px] italic text-[var(--color-muted-foreground)]">
              The model registry is empty. This shouldn't happen — please file an issue.
            </p>
          </Show>

          <Show when={modelsStore.state.models.length > 0}>
            <div class="space-y-2">
              <For each={modelsStore.state.models}>
                {(model) => <ModelDownloadCard model={model} />}
              </For>
            </div>
          </Show>
        </section>

        {/* Search quality — collapsed from 4 confusing toggles to one pill */}
        <section class="material-linen p-5">
          <h2 class="mb-1 text-sm font-semibold text-embossed">Search Quality</h2>
          <p class="mb-4 text-xs leading-relaxed text-[var(--color-foreground-muted)]">
            How hard the app works to rank your results. The keyword baseline
            (TF-IDF, RRF, authority) always runs.
          </p>

          <div class="surface-raised-sm flex items-center gap-1 p-1 mb-3">
            <QualityTab
              q="fast"
              label="Fast"
              caption="Lexical only · instant"
              active={settings.quality === "fast"}
            />
            <QualityTab
              q="balanced"
              label="Balanced"
              caption={modelsStore.embeddingReady ? "+ semantic rerank · ~5s" : "needs embedding model"}
              active={settings.quality === "balanced"}
              disabled={!modelsStore.embeddingReady}
            />
            <QualityTab
              q="thorough"
              label="Thorough"
              caption={modelsStore.llmReady ? "+ LLM expand & filter · slower" : "needs LLM model"}
              active={settings.quality === "thorough"}
              disabled={!modelsStore.llmReady}
            />
          </div>

          <p class="mb-4 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
            {settings.quality === "fast" && "Keyword scoring across all sources. Returns immediately. No models needed."}
            {settings.quality === "balanced" && "Adds semantic reranking via the embedding model — top 100 results re-scored by query meaning, not just keyword overlap."}
            {settings.quality === "thorough" && "Full AI pipeline: LLM generates extra sub-queries before discovery, semantic reranking, then LLM borderline-filter pass on the middle band. Several seconds slower."}
          </p>

          <RankingToggle
            checked={settings.useCitationGraph}
            onToggle={(v) => {
              setSettings("useCitationGraph", v);
              saveSettings();
            }}
            label="Citation-graph reasoning"
            detail="Cross-references Semantic Scholar — boosts papers cited by other top results. Slow."
          />
        </section>

        {/* Library folder */}
        <section class="material-paper border-stitched p-5">
          <h2 class="mb-3 text-sm font-semibold text-embossed">Library Folder</h2>
          <label>
            <span class="sr-only">Library folder path</span>
            <input
              type="text"
              value={settings.libraryRoot}
              onInput={(e) => {
                setSettings("libraryRoot", e.currentTarget.value);
                saveSettings();
              }}
              class="surface-input w-full px-3 py-2 font-mono text-xs outline-none"
            />
          </label>
        </section>

        {/* Web search */}
        <section class="material-linen p-5">
          <h2 class="mb-1 text-sm font-semibold text-embossed">Web Search (no setup required)</h2>
          <p class="mb-3 text-xs leading-relaxed text-[var(--color-foreground-muted)]">
            Searches multiple web indexes in parallel — no Docker, no API keys,
            no accounts needed.
          </p>
          <div class="surface-pressed-sm flex items-start gap-2 p-3 text-xs leading-relaxed mb-3">
            <CheckCircle2 size={14} class="mt-0.5 shrink-0" style={{ color: "var(--color-success)" }} />
            <span>
              DuckDuckGo · Brave · Bing · Mojeek · Marginalia · Startpage
            </span>
          </div>
          <MetaSearchHealthBar />
        </section>

        {/* Run log */}
        <section class="material-paper border-stitched p-5">
          <h2 class="mb-1 text-sm font-semibold text-embossed">Run Log</h2>
          <p class="mb-4 text-xs text-[var(--color-muted-foreground)]">
            Every query, source error, and download outcome is logged here.
            Share this file when reporting issues.
          </p>
          <Show
            when={logInfo()}
            fallback={<p class="text-xs text-[var(--color-muted-foreground)]">Unavailable</p>}
          >
            {(info) => (
              <div class="space-y-3">
                <code
                  title={info().path}
                  class="surface-pressed-sm block truncate px-3 py-2 font-mono text-[11px]"
                >
                  {info().path}
                </code>
                <p class="text-xs text-[var(--color-muted-foreground)]">
                  {info().exists ? formatBytes(info().size_bytes) : "Not yet written"}
                </p>
                <div class="flex gap-2">
                  <button
                    onClick={() => api.revealInFinder(info().path)}
                    disabled={!info().exists}
                    class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
                  >
                    <FolderOpen size={12} />
                    Show in Finder
                  </button>
                  <button
                    onClick={async () => setLogInfo(await api.runLogInfo().catch(() => null))}
                    class="flex items-center gap-1.5 rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-xs font-medium hover:bg-[var(--color-accent)] transition-colors"
                  >
                    <FileText size={12} />
                    Refresh
                  </button>
                </div>
              </div>
            )}
          </Show>
        </section>
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
  disabledHint?: string;
}) {
  return (
    <label
      class="flex items-start gap-2 text-xs"
      classList={{ "opacity-50": !!props.disabled }}
    >
      <input
        type="checkbox"
        checked={props.checked && !props.disabled}
        disabled={!!props.disabled}
        onChange={(e) => props.onToggle(e.currentTarget.checked)}
        class="mt-0.5 h-3.5 w-3.5 rounded border-[var(--color-border)]"
      />
      <span class="flex-1">
        <span class="font-medium">{props.label}</span>{" "}
        <span class="text-[10px] text-[var(--color-muted-foreground)]">
          · {props.detail}
        </span>
        <Show when={props.disabled && props.disabledHint}>
          <p class="mt-0.5 text-[10px] italic text-[var(--color-muted-foreground)]">
            {props.disabledHint}
          </p>
        </Show>
      </span>
    </label>
  );
}

/// One of three quality pills. Stacks label + caption vertically so the
/// per-state explanation lives directly under the choice without needing
/// a tooltip. Disabled when the prerequisite model isn't downloaded yet.
function QualityTab(props: {
  q: Quality;
  label: string;
  caption: string;
  active: boolean;
  disabled?: boolean;
}) {
  // pill-toggle's class default handles inactive coloring; only active
  // overrides to indigo primary (label full strength, caption slightly
  // softer).
  return (
    <button
      onClick={() => {
        if (props.disabled) return;
        setSettings("quality", props.q);
        saveSettings();
      }}
      disabled={props.disabled}
      class="pill-toggle flex-1 px-3 py-2 text-center"
      classList={{ "is-active": props.active, "opacity-55 cursor-not-allowed": !!props.disabled }}
    >
      <div
        class="text-[12px] font-semibold leading-tight"
        style={props.active ? { color: "var(--color-primary)" } : {}}
      >
        {props.label}
      </div>
      <div
        class="mt-0.5 text-[10px] leading-tight"
        style={
          props.active
            ? { color: "color-mix(in oklch, var(--color-primary) 75%, transparent)" }
            : { color: "var(--color-foreground-muted)" }
        }
      >
        {props.caption}
      </div>
    </button>
  );
}
