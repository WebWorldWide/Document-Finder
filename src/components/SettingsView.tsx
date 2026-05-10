import { createSignal, onMount, Show, For } from "solid-js";
import { FolderOpen, FileText, Loader2, CheckCircle2, Sparkles, RefreshCw, AlertCircle } from "lucide-solid";
import { api, type LogInfo } from "@/lib/tauri";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import { modelsStore } from "@/stores/models";
import ModelDownloadCard from "./ModelDownloadCard";
import SearxngSetupPanel from "./SearxngSetupPanel";
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
          <div class="grid grid-cols-3 gap-4">
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
          <div class="mb-3 flex items-center gap-2">
            <Sparkles size={14} class="text-[var(--color-primary)]" />
            <h2 class="text-sm font-semibold text-embossed">AI Models</h2>
            <Show when={modelsStore.totalDiskBytes > 0}>
              <span class="ml-auto text-[10px] text-[var(--color-muted-foreground)]">
                {formatBytes(modelsStore.totalDiskBytes)} on disk
              </span>
            </Show>
          </div>
          <p class="mb-4 text-xs leading-relaxed text-[var(--color-muted-foreground)]">
            Two local models power Tier 2 (semantic reranking) and Tier 3
            (LLM query expansion + borderline filtering). Everything runs
            offline — no API keys, no telemetry. Models can be deleted any
            time to reclaim disk.
          </p>
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

        {/* Ranking */}
        <section class="material-feltgreen p-5">
          <h2 class="mb-1 text-sm font-semibold text-embossed-on-dark">Ranking</h2>
          <p class="mb-4 text-xs text-[var(--color-muted-foreground)]">
            Cross-source dedup, TF-IDF, and Reciprocal Rank Fusion are always on.
            The toggles below add additional ranking signals.
          </p>
          <div class="space-y-2.5">
            <RankingToggle
              checked={settings.useSemanticRerank}
              onToggle={(v) => {
                setSettings("useSemanticRerank", v);
                saveSettings();
              }}
              label="Semantic reranking"
              detail="bge-small-en-v1.5"
              disabled={!modelsStore.embeddingReady}
              disabledHint="Download the embedding model above to enable."
            />
            <RankingToggle
              checked={settings.useLlmExpansion}
              onToggle={(v) => {
                setSettings("useLlmExpansion", v);
                saveSettings();
              }}
              label="LLM query expansion"
              detail="generates 5–8 alternative search phrasings before discovery"
              disabled={!modelsStore.llmReady}
              disabledHint="Download an LLM model above to enable."
            />
            <RankingToggle
              checked={settings.useLlmFilter}
              onToggle={(v) => {
                setSettings("useLlmFilter", v);
                saveSettings();
              }}
              label="LLM borderline filter"
              detail="judges 50–70th percentile candidates for topical fit"
              disabled={!modelsStore.llmReady}
              disabledHint="Download an LLM model above to enable."
            />
            <RankingToggle
              checked={settings.useCitationGraph}
              onToggle={(v) => {
                setSettings("useCitationGraph", v);
                saveSettings();
              }}
              label="Citation-graph reasoning"
              detail="Semantic Scholar refs/cites — slower, deeper"
            />
          </div>
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
          <h2 class="mb-1 text-sm font-semibold text-embossed">Web Search</h2>
          <p class="mb-4 text-xs text-[var(--color-muted-foreground)]">
            Document-Finder includes a built-in meta-search across DuckDuckGo,
            Brave, Bing, Mojeek, Marginalia, and Startpage — no Docker, no
            setup, no API keys. It's enabled by default in Discover.
          </p>
          <div class="surface-pressed-sm mb-4 p-3 text-xs leading-relaxed">
            <p class="flex items-start gap-2">
              <CheckCircle2 size={14} class="mt-0.5 shrink-0" style={{ color: "var(--color-success)" }} />
              <span>
                <span class="font-medium">Built-in meta-search is active.</span>
                {" "}Six independent engines are queried in parallel and
                deduped into a single result stream. Toggle individual
                engines below if you want to narrow it down.
              </span>
            </p>
          </div>

          <details class="surface-raised-subtle">
            <summary class="cursor-pointer px-3 py-2 text-xs font-medium text-[var(--color-foreground-muted)] hover:text-[var(--color-foreground)]">
              Advanced: SearXNG (public instance or local Docker)
            </summary>
            <div class="space-y-4 px-3 pb-3 pt-1">
              <p class="text-[11px] text-[var(--color-foreground-muted)]">
                SearXNG aggregates dozens more engines than the built-in
                set. Most users don't need it.
              </p>

              <label class="block">
                <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">
                  SearXNG instance URL
                </span>
                <input
                  type="text"
                  value={settings.searxngUrl}
                  onInput={(e) => {
                    setSettings("searxngUrl", e.currentTarget.value);
                    saveSettings();
                  }}
                  placeholder="https://searx.be"
                  class="surface-input w-full px-3 py-2 font-mono text-xs outline-none"
                />
                <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">
                  Paste any public instance from{" "}
                  <a
                    href="https://searx.space"
                    target="_blank"
                    rel="noreferrer"
                    class="underline hover:text-[var(--color-primary)]"
                  >
                    searx.space
                  </a>
                  {" "}— or pick one below.
                </p>
              </label>

              <div class="flex flex-wrap gap-1.5">
                <For
                  each={[
                    "https://searx.be",
                    "https://searx.tiekoetter.com",
                    "https://search.disroot.org",
                    "https://priv.au",
                  ]}
                >
                  {(url) => {
                    const active = () => settings.searxngUrl === url;
                    return (
                      <button
                        onClick={() => {
                          setSettings("searxngUrl", url);
                          saveSettings();
                        }}
                        class="tag-pill px-2.5 py-0.5 text-[10px] font-mono"
                        classList={{ "is-active": active() }}
                        style={
                          active()
                            ? { "background-color": "var(--color-source-searxng)", color: "white" }
                            : { color: "var(--color-foreground-muted)" }
                        }
                      >
                        {url.replace("https://", "")}
                      </button>
                    );
                  }}
                </For>
              </div>

              <details>
                <summary class="cursor-pointer text-[11px] font-medium text-[var(--color-foreground-muted)]">
                  Spin up a local instance with Docker
                </summary>
                <div class="pt-2">
                  <p class="mb-2 text-[11px] text-[var(--color-foreground-muted)]">
                    Requires Docker installed and running.
                  </p>
                  <SearxngSetupPanel />
                </div>
              </details>
            </div>
          </details>
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
