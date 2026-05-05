import { For, Show, createMemo } from "solid-js";
import { CheckCircle2, XCircle } from "lucide-solid";
import { runStore } from "@/stores/run";
import { formatBytes, SOURCE_LABELS, sourceColor } from "@/lib/utils";

export default function LiveDownloadStream() {
  const inFlightList = createMemo(() => Object.values(runStore.state.inFlight));
  const recentItems = createMemo(() => runStore.state.completed.slice(-50).reverse());

  return (
    <div class="space-y-4">
      {/* In-flight downloads */}
      <Show when={inFlightList().length > 0}>
        <div>
          <p class="mb-2 text-[11px] font-medium uppercase tracking-wider text-[var(--color-muted-foreground)]">
            Downloading ({inFlightList().length})
          </p>
          <div class="space-y-2">
            <For each={inFlightList()}>
              {(item) => {
                const pct = () =>
                  item.total > 0 ? Math.round((item.downloaded / item.total) * 100) : 0;
                return (
                  <div class="animate-slide-in rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-3">
                    <div class="flex items-center justify-between gap-2 mb-2">
                      <span
                        class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium text-white"
                        style={{ "background-color": sourceColor(item.source) }}
                      >
                        {SOURCE_LABELS[item.source] ?? item.source}
                      </span>
                      <span class="truncate text-xs text-[var(--color-muted-foreground)] flex-1 text-left">
                        {item.title}
                      </span>
                    </div>
                    <div class="flex items-center gap-2">
                      <div class="flex-1 h-1 rounded-full bg-[var(--color-border)] overflow-hidden">
                        <div
                          class="h-full rounded-full transition-all duration-300"
                          classList={{
                            "progress-shimmer": item.total === 0,
                            "bg-[var(--color-primary)]": item.total > 0,
                          }}
                          style={{ width: item.total === 0 ? "100%" : `${pct()}%` }}
                        />
                      </div>
                      <span class="shrink-0 text-[10px] text-[var(--color-muted-foreground)]">
                        {item.total > 0
                          ? `${formatBytes(item.downloaded)} / ${formatBytes(item.total)}`
                          : formatBytes(item.downloaded)}
                      </span>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </div>
      </Show>

      {/* Recent completions */}
      <Show when={recentItems().length > 0}>
        <div>
          <p class="mb-2 text-[11px] font-medium uppercase tracking-wider text-[var(--color-muted-foreground)]">
            Recent
          </p>
          <div class="space-y-1">
            <For each={recentItems()}>
              {(item) => (
                <div class="animate-slide-in flex items-center gap-2 rounded-md px-2 py-1.5">
                  <Show
                    when={item.status === "done"}
                    fallback={<XCircle size={13} class="shrink-0 text-[var(--color-destructive)]" />}
                  >
                    <CheckCircle2 size={13} class="shrink-0" style={{ color: "var(--color-success)" }} />
                  </Show>
                  <span
                    class="shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-medium text-white"
                    style={{ "background-color": sourceColor(item.source) }}
                  >
                    {SOURCE_LABELS[item.source] ?? item.source}
                  </span>
                  <span class="flex-1 truncate text-[11px]">{item.title}</span>
                  <Show when={item.error}>
                    <span class="shrink-0 text-[10px] text-[var(--color-destructive)] max-w-24 truncate">
                      {item.error}
                    </span>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}
