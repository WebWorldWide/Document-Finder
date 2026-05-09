import { createSignal, onMount, Show, For } from "solid-js";
import { Server, FolderOpen, FileText, Loader2, CheckCircle2 } from "lucide-solid";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type LogInfo } from "@/lib/tauri";
import { settings, setSettings, saveSettings } from "@/stores/settings";
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
        <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
          <h2 class="mb-4 text-sm font-semibold">Discovery</h2>
          <div class="grid grid-cols-3 gap-4">
            <label class="block">
              <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">Per source</span>
              <input
                type="number"
                min="1"
                value={settings.perSource}
                onInput={numInput("perSource")}
                class="w-full rounded-lg border border-[var(--color-border)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
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
                class="w-full rounded-lg border border-[var(--color-border)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
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
                class="w-full rounded-lg border border-[var(--color-border)] bg-transparent px-3 py-2 text-sm outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
              />
              <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">Higher = faster but more rate limits</p>
            </label>
          </div>
        </section>

        {/* Library folder */}
        <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
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
              class="w-full rounded-lg border border-[var(--color-border)] bg-transparent px-3 py-2 font-mono text-xs outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
            />
          </label>
        </section>

        {/* SearXNG */}
        <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
          <h2 class="mb-1 text-sm font-semibold">Search Infrastructure</h2>
          <p class="mb-4 text-xs text-[var(--color-muted-foreground)]">
            SearXNG is an optional, privacy-respecting metasearch engine. The app
            already searches the web via DuckDuckGo, Brave, and Bing scrapers
            without any setup — SearXNG adds dozens more engines if you have Docker.
          </p>
          <div class="space-y-4">
            <button
              onClick={handleSetupSearx}
              disabled={settingUpSearx()}
              class="flex items-center gap-2 rounded-lg border border-[var(--color-border)] px-4 py-2 text-sm font-medium hover:bg-[var(--color-accent)] transition-colors disabled:opacity-50"
            >
              <Show when={settingUpSearx()} fallback={<Server size={14} />}>
                <Loader2 size={14} class="animate-spin" />
              </Show>
              {settingUpSearx() ? "Setting up… (may take a few minutes)" : "Setup SearXNG with Docker"}
            </button>

            <label class="block">
              <span class="mb-1 block text-xs font-medium text-[var(--color-muted-foreground)]">SearXNG instance URL</span>
              <input
                type="text"
                value={settings.searxngUrl}
                onInput={(e) => {
                  setSettings("searxngUrl", e.currentTarget.value);
                  saveSettings();
                }}
                placeholder="http://localhost:8080"
                class="w-full rounded-lg border border-[var(--color-border)] bg-transparent px-3 py-2 font-mono text-xs outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
              />
              <p class="mt-1 text-[10px] text-[var(--color-muted-foreground)]">
                Local or remote SearXNG instance
              </p>
            </label>

            <Show when={searxStage()}>
              {(stage) => (
                <div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-muted)] p-3 text-xs">
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
        </section>

        {/* Run log */}
        <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
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
                  class="block truncate rounded-lg border border-[var(--color-border)] bg-[var(--color-muted)] px-3 py-2 font-mono text-[11px]"
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
                    class="flex items-center gap-1.5 rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-xs font-medium hover:bg-[var(--color-accent)] transition-colors disabled:opacity-40"
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
