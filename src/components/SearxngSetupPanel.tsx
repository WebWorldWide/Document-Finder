import { createSignal, onCleanup, Show, For } from "solid-js";
import { Server, Loader2, CheckCircle2 } from "lucide-solid";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "@/lib/tauri";
import { setSettings, saveSettings } from "@/stores/settings";

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

/// Reusable SearXNG-via-Docker setup UI.
///
/// Owns its own listener subscriptions for the streaming setup events
/// (`df:searxng_setup_log`, `df:searxng_setup_stage`) so it can be mounted
/// in multiple views (Settings, WelcomeDialog) without coordination.
///
/// Persists the resolved instance URL into `settings.searxngUrl` on
/// success so subsequent searches use it automatically. Calls
/// `props.onComplete?.(url)` so a host can react (e.g. dismiss a dialog).
export default function SearxngSetupPanel(props: {
  onComplete?: (url: string) => void;
  /// Optional surrounding spacing/typography overrides supplied by the host.
  compact?: boolean;
}) {
  const [running, setRunning] = createSignal(false);
  const [result, setResult] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [log, setLog] = createSignal<SearxLogLine[]>([]);
  const [stage, setStage] = createSignal<SearxStage | null>(null);

  let unsubs: UnlistenFn[] = [];
  onCleanup(() => unsubs.forEach((u) => u()));

  async function start() {
    setRunning(true);
    setResult(null);
    setError(null);
    setLog([]);
    setStage(null);

    unsubs.push(
      await listen<SearxLogLine>("df:searxng_setup_log", (ev) => {
        setLog((prev) => {
          const next = prev.concat(ev.payload);
          return next.length > 500 ? next.slice(next.length - 500) : next;
        });
      }),
    );
    unsubs.push(
      await listen<SearxStage>("df:searxng_setup_stage", (ev) => {
        setStage(ev.payload);
      }),
    );

    try {
      const output = await api.setupSearXNG();
      setResult(output);
      const match = output.match(/^SEARXNG_URL=(.+)$/m);
      if (match) {
        const url = match[1].trim();
        setSettings("searxngUrl", url);
        saveSettings();
        props.onComplete?.(url);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      // Keep listeners around so trailing log lines after the command
      // resolves still arrive — they'll be torn down on component unmount.
    }
  }

  return (
    <div class={props.compact ? "space-y-2" : "space-y-3"}>
      <button
        onClick={start}
        disabled={running()}
        class="btn-tactile flex items-center gap-2 px-3 py-1.5 text-xs font-medium"
      >
        <Show when={running()} fallback={<Server size={12} />}>
          <Loader2 size={12} class="animate-spin" />
        </Show>
        {running() ? "Setting up…" : "Setup SearXNG with Docker"}
      </button>

      <Show when={stage()}>
        {(s) => (
          <div class="surface-pressed-sm p-3 text-xs">
            <div class="flex items-center gap-2 font-medium">
              <Show
                when={s().stage !== "ok" && s().stage !== "failed"}
                fallback={
                  <Show
                    when={s().stage === "ok"}
                    fallback={<span class="text-[var(--color-destructive)]">●</span>}
                  >
                    <CheckCircle2 size={12} style={{ color: "var(--color-success)" }} />
                  </Show>
                }
              >
                <Loader2 size={12} class="animate-spin" />
              </Show>
              <span>{STAGE_LABEL[s().stage]}</span>
              <Show when={s().detail}>
                <span class="font-mono text-[10px] text-[var(--color-muted-foreground)]">
                  {s().detail}
                </span>
              </Show>
            </div>
          </div>
        )}
      </Show>

      <Show when={log().length > 0}>
        <div class="rounded-lg border border-[var(--color-border)] bg-black/90 p-2">
          <pre class="max-h-40 overflow-auto whitespace-pre-wrap font-mono text-[10px] text-green-400/90 leading-snug">
            <For each={log()}>
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

      <Show when={result() && !error()}>
        <div
          class="flex items-start gap-2 rounded-lg border p-3 text-xs"
          style={{
            "border-color": "color-mix(in oklch, var(--color-success) 30%, transparent)",
            "background-color": "var(--color-success-bg)",
          }}
        >
          <CheckCircle2 size={14} class="mt-0.5 shrink-0" style={{ color: "var(--color-success)" }} />
          <div>
            <p class="font-medium" style={{ color: "var(--color-success-fg)" }}>
              SearXNG is running.
            </p>
            <pre class="mt-1 max-h-24 overflow-auto whitespace-pre-wrap font-mono text-[10px] opacity-80" style={{ color: "var(--color-success-fg)" }}>
              {result()}
            </pre>
          </div>
        </div>
      </Show>

      <Show when={error()}>
        <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-3 text-xs text-[var(--color-destructive)]">
          <p class="font-medium">Setup failed</p>
          <p class="mt-0.5 opacity-80 whitespace-pre-wrap">{error()}</p>
        </div>
      </Show>
    </div>
  );
}
