import { Show, createMemo } from "solid-js";
import { Brain, Sparkles, Loader2 } from "lucide-solid";
import { modelsStore } from "@/stores/models";

/// Compact pill that shows what the AI is doing during a search run.
/// Surfaces the EV_MODEL_STATUS activity events from the backend so the user
/// can see "LLM expanding query" or "Embedding 23/100" instead of staring
/// at a silent UI while a model warms.
export default function ModelStatusBadge() {
  const activities = createMemo(() =>
    Object.entries(modelsStore.state.activity).map(([id, a]) => ({ id, ...a }))
  );

  return (
    <Show when={activities().length > 0}>
      <div class="flex items-center gap-1.5">
        {activities().map((a) => (
          <span class="inline-flex items-center gap-1 rounded-full border border-[var(--color-border)] bg-[var(--color-card)] px-2 py-0.5 text-[10px] font-medium">
            <Show
              when={a.status === "embedding"}
              fallback={
                <Show
                  when={a.status === "llm_warming"}
                  fallback={<Brain size={10} class="text-amber-500" />}
                >
                  <Loader2 size={10} class="animate-spin text-amber-500" />
                </Show>
              }
            >
              <Sparkles size={10} class="text-[var(--color-primary)]" />
            </Show>
            <span class="text-[var(--color-foreground)]">{labelFor(a.status)}</span>
            <Show when={a.detail}>
              <span class="text-[var(--color-muted-foreground)]">{a.detail}</span>
            </Show>
          </span>
        ))}
      </div>
    </Show>
  );
}

function labelFor(status: string): string {
  switch (status) {
    case "embedding":
      return "Embedding";
    case "llm_warming":
      return "LLM warming";
    case "llm_expanding":
      return "Expanding";
    case "llm_filtering":
      return "Filtering";
    default:
      return status;
  }
}
