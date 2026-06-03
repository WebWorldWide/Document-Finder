import { createSignal, onCleanup, onMount, For, Show } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface MetaSearchHealthPayload {
  backend: string;
  // "empty" (healthy, zero results) and "partial" (slow but returned results)
  // are non-failing states the backend emits so they don't trip the breaker.
  status: "ok" | "empty" | "partial" | "timeout" | "circuit_open" | "error";
  result_count: number;
  latency_ms: number;
}

interface BackendStatus extends MetaSearchHealthPayload {
  ts: number;
}

const BACKEND_LABELS: Record<string, string> = {
  duckduckgo: "DuckDuckGo",
  brave: "Brave",
  bing: "Bing",
  mojeek: "Mojeek",
  marginalia: "Marginalia",
  startpage: "Startpage",
};

function statusColor(status: BackendStatus["status"]): string {
  switch (status) {
    case "ok":
      return "var(--color-success)";
    case "empty":
      return "var(--ink-4, oklch(0.7 0 0))";
    case "partial":
    case "timeout":
      return "var(--color-warning, oklch(0.75 0.15 80))";
    case "circuit_open":
    case "error":
      return "var(--color-destructive)";
  }
}

export default function MetaSearchHealthBar() {
  const [backends, setBackends] = createSignal<BackendStatus[]>([]);

  // Register onCleanup SYNCHRONOUSLY in the component body (not after the await
  // inside onMount — by then the owner scope has exited and the cleanup would
  // never run, leaking the Tauri listener across remounts).
  let unlisten: UnlistenFn | undefined;
  // Guard against unmount during the await below: if the component is disposed
  // before listen() resolves, onCleanup runs while `unlisten` is still
  // undefined (a no-op), and the handle that resolves afterwards would leak. The
  // `disposed` flag lets us tear it down immediately instead.
  let disposed = false;
  onCleanup(() => {
    disposed = true;
    unlisten?.();
  });

  onMount(async () => {
    const u = await listen<MetaSearchHealthPayload>("df:meta_search_health", (ev) => {
      const payload = ev.payload;
      setBackends((prev) => {
        const idx = prev.findIndex((b) => b.backend === payload.backend);
        const entry: BackendStatus = { ...payload, ts: Date.now() };
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = entry;
          return next;
        }
        return [...prev, entry];
      });
    });
    if (disposed) u();
    else unlisten = u;
  });

  return (
    <Show when={backends().length > 0}>
      <div class="space-y-1">
        <p class="mb-1.5 text-[10px] font-medium text-[var(--color-muted-foreground)]">
          Backend health (last search)
        </p>
        <div class="flex flex-wrap gap-1.5">
          <For each={backends()}>
            {(b) => (
              <div
                class="flex items-center gap-1 rounded px-2 py-0.5 text-[10px]"
                style={{
                  background: "var(--color-surface-raised, oklch(0.97 0 0))",
                  border: `1px solid ${statusColor(b.status)}`,
                }}
                title={`${b.result_count} results · ${b.latency_ms}ms`}
              >
                <span
                  class="h-1.5 w-1.5 rounded-full"
                  style={{ background: statusColor(b.status) }}
                />
                <span class="font-medium">{BACKEND_LABELS[b.backend] ?? b.backend}</span>
                <Show when={b.status === "ok" || b.status === "partial"}>
                  <span class="text-[var(--color-foreground-muted)]">{b.result_count}</span>
                </Show>
                <Show when={b.status !== "ok"}>
                  <span style={{ color: statusColor(b.status) }}>
                    {b.status === "circuit_open"
                      ? "blocked"
                      : b.status === "empty"
                        ? "none"
                        : b.status}
                  </span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}
