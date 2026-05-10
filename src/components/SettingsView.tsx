import { createSignal, onMount, Show, For } from "solid-js";
import { Server, FolderOpen, FileText, Loader2, CheckCircle2, Sparkles } from "lucide-solid";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type LogInfo } from "@/lib/tauri";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import { modelsStore } from "@/stores/models";
import ModelDownloadCard from "./ModelDownloadCard";
import { formatBytes } from "@/lib/utils";

interface SearxLogLine {
  stream: "stdout" | "stderr" | "info";
  line: string;
}
interface SearxStage {
  stage:
    | "checking_docker"
    | "checking_port"
    | "pulling"
    | "starting"
    | "waiting_health"
    | "ok"
    | "failed";
  detail?: string | null;
}

const STAGE_LABEL: Record<SearxStage["stage"], string> = {
  checking_docker: "Checking Docker…",
  checking_port: "Checking port availability…",
  pulling: "Pulling SearXNG image…",
  starting: "Starting container…",
  waiting_health: "Waiting for JSON health check…",
  ok: "Healthy",
  failed: "Failed",
};

export default function SettingsView() {
  const [logInfo, setLogInfo] = createSignal<LogInfo | null>(null);
  const [settingUpSearx, setSettingUpSearx] = createSignal(false);
  const [searxResult, setSearxResult] = createSignal<string | null>(null);
  const [searxError, setSearxError] = createSignal<string | null>(null);
  const [searxLog, setSearxLog] = createSignal<SearxLogLine[]>([]);
  const [searxStage, setSearxStage] = createSignal<SearxStage | null>(null);

  onMount(async () => {
    setLogInfo(await api.runLogInfo().catch(() => null));
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  async function handleSetupSearx() {
    setSettingUpSearx(true);
    setSearxResult(null);
    setSearxError(null);
    setSearxLog([]);
    setSearxStage(null);

    const unsubs: UnlistenFn[] = [];
    unsubs.push(
      await listen<SearxLogLine>("df:searxng_setup_log", (ev) => {
        setSearxLog((prev) => {
          const next = prev.concat(ev.payload);
          // Cap to last 500 lines so the modal doesn't grow unbounded.
          return next.length > 500 ? next.slice(next.length - 500) : next;
        });
      }),
    );
    unsubs.push(
      await listen<SearxStage>("df:searxng_setup_stage", (ev) => {
        setSearxStage(ev.payload);
      }),
    );

    try {
      const output = await api.setupSearXNG();
      setSearxResult(output);
      const match = output.match(/^SEARXNG_URL=(.+)$/m);
      if (match) {
        setSettings("searxngUrl", match[1].trim());
        saveSettings();
      }
    } catch (e) {
      setSearxError(String(e));
    } finally {
      setSettingUpSearx(false);
      unsubs.forEach((u) => u());
    }
  }

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
        <h1 class="text-xl font-semibold">Settings</h1>

        {/* Discovery settings */}
        <section class="surface-raised p-5">
          <h2 class="mb-4 text-sm font-semibold">Discovery</h2>
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
        <section class="surface-raised p-5">
          <div class="mb-3 flex items-center gap-2">
            <Sparkles size={14} class="text-[var(--color-primary)]" />
            <h2 class="text-sm font-semibold">AI Models</h2>
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
          <Show
            when={modelsStore.state.models.length > 0}
            fallback={
              <p class="text-[11px] text-[var(--color-muted-foreground)]">
                Loading…
              </p>
            }
          >
            <div class="space-y-2">
              <For each={modelsStore.state.models}>
                {(model) => <ModelDownloadCard model={model} />}
              </For>
            </div>
          </Show>
        </section>

        {/* Ranking */}
        <section class="surface-raised p-5">
          <h2 class="mb-1 text-sm font-semibold">Ranking</h2>
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
        <section class="surface-raised p-5">
          <h2 class="mb-3 text-sm font-semibold">Library Folder</h2>
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
        <section class="surface-raised p-5">
          <h2 class="mb-1 text-sm font-semibold">Web Search</h2>
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
                <div class="space-y-3 pt-2">
                  <p class="text-[11px] text-[var(--color-foreground-muted)]">
                    Requires Docker installed and running.
                  </p>
                  <button
                    onClick={handleSetupSearx}
                    disabled={settingUpSearx()}
                    class="btn-tactile flex items-center gap-2 px-3 py-1.5 text-xs font-medium"
                  >
                    <Show when={settingUpSearx()} fallback={<Server size={12} />}>
                      <Loader2 size={12} class="animate-spin" />
                    </Show>
                    {settingUpSearx() ? "Setting up…" : "Setup SearXNG with Docker"}
                  </button>

                  <Show when={searxStage()}>
                    {(stage) => (
                      <div class="surface-pressed-sm p-3 text-xs">
                        <div class="flex items-center gap-2 font-medium">
                          <Show
                            when={stage().stage !== "ok" && stage().stage !== "failed"}
                            fallback={
                              <Show
                                when={stage().stage === "ok"}
                                fallback={<span class="text-[var(--color-destructive)]">●</span>}
                              >
                                <CheckCircle2 size={12} style={{ color: "var(--color-success)" }} />
                              </Show>
                            }
                          >
                            <Loader2 size={12} class="animate-spin" />
                          </Show>
                          <span>{STAGE_LABEL[stage().stage]}</span>
                          <Show when={stage().detail}>
                            <span class="font-mono text-[10px] text-[var(--color-muted-foreground)]">
                              {stage().detail}
                            </span>
                          </Show>
                        </div>
                      </div>
                    )}
                  </Show>

                  <Show when={searxLog().length > 0}>
                    <div class="rounded-lg border border-[var(--color-border)] bg-black/90 p-2">
                      <pre class="max-h-48 overflow-auto whitespace-pre-wrap font-mono text-[10px] text-green-400/90 leading-snug">
                        <For each={searxLog()}>
                          {(line) => (
                            <div
                              class={
                                line.stream === "stderr"
                                  ? "text-red-300/90"
                                  : line.stream === "info"
                                  ? "text-cyan-300/80"
                                  : ""
                              }
                            >
                              {line.line}
                            </div>
                          )}
                        </For>
                      </pre>
                    </div>
                  </Show>

                  <Show when={searxResult() !== null && !searxError()}>
                    <div class="flex items-start gap-2 rounded-lg border p-3"
                      style={{ "border-color": "color-mix(in oklch, var(--color-success) 30%, transparent)", "background-color": "var(--color-success-bg)" }}
                    >
                      <CheckCircle2 size={14} class="mt-0.5 shrink-0" style={{ color: "var(--color-success)" }} />
                      <div>
                        <p class="text-xs font-medium" style={{ color: "var(--color-success-fg)" }}>SearXNG is running at {settings.searxngUrl}</p>
                        <Show when={searxResult()}>
                          <pre class="mt-1 max-h-24 overflow-auto whitespace-pre-wrap font-mono text-[10px] opacity-80" style={{ color: "var(--color-success-fg)" }}>
                            {searxResult()}
                          </pre>
                        </Show>
                      </div>
                    </div>
                  </Show>

                  <Show when={searxError()}>
                    <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-3 text-xs text-[var(--color-destructive)]">
                      <p class="font-medium">Setup failed</p>
                      <p class="mt-0.5 opacity-80 whitespace-pre-wrap">{searxError()}</p>
                    </div>
                  </Show>
                </div>
              </details>
            </div>
          </details>
        </section>

        {/* Run log */}
        <section class="surface-raised p-5">
          <h2 class="mb-1 text-sm font-semibold">Run Log</h2>
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
