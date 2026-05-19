import { createSignal, onCleanup, onMount, For, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";

interface MetaSearchHealthPayload {
  backend: string;
  status: "ok" | "timeout" | "circuit_open" | "error";
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
    case "timeout":
      return "var(--color-warning, oklch(0.75 0.15 80))";
    case "circuit_open":
      return "var(--color-destructive)";
    case "error":
      return "var(--color-destructive)";
  }
}

export default function MetaSearchHealthBar() {
  const [backends, setBackends] = createSignal<BackendStatus[]>([]);

  onMount(async () => {
    const unlisten = await listen<MetaSearchHealthPayload>("df:meta_search_health", (ev) => {
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
    onCleanup(() => unlisten());
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
                <Show when={b.status === "ok"}>
                  <span class="text-[var(--color-foreground-muted)]">{b.result_count}</span>
                </Show>
                <Show when={b.status !== "ok"}>
                  <span style={{ color: statusColor(b.status) }}>
                    {b.status === "circuit_open" ? "blocked" : b.status}
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
