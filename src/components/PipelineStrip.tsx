import { For, Show, onMount, createMemo } from "solid-js";
import {
  Sparkles,
  Search,
  ListOrdered,
  Brain,
  Filter,
  GitBranch,
  Download,
  FileText,
  Check,
  Loader2,
  Minus,
} from "lucide-solid";
import { pipelineStore } from "@/stores/pipeline";
import type { PipelineStage } from "@/lib/events";

const STAGE_META: Record<
  PipelineStage,
  { label: string; icon: (props: { size?: number; class?: string }) => any }
> = {
  llm_expand: { label: "Expand", icon: Sparkles },
  discovery: { label: "Discover", icon: Search },
  rank: { label: "Rank", icon: ListOrdered },
  semantic_rerank: { label: "Embed", icon: Brain },
  llm_filter: { label: "Filter", icon: Filter },
  citation_enrich: { label: "Cites", icon: GitBranch },
  download: { label: "Download", icon: Download },
  extract: { label: "Extract", icon: FileText },
};

export default function PipelineStrip() {
  onMount(() => {
    void pipelineStore.ensureSubscribed();
  });

  // Show a small subtitle with which stages are still active.
  const activeCount = createMemo(() => {
    const stages = pipelineStore.stages;
    return Object.values(stages).filter(
      (s) => s.state === "started" || s.state === "progress"
    ).length;
  });

  return (
    <div class="surface-raised-sm surface-bevel-sm mx-4 mb-2 flex items-center gap-2 px-3 py-2">
      <span class="text-[9px] font-semibold uppercase tracking-wider text-[var(--color-foreground-muted)] text-embossed shrink-0">
        Pipeline
      </span>
      <div class="flex flex-1 items-center gap-1 overflow-x-auto">
        <For each={pipelineStore.ordered}>
          {(stage) => <StageTile stage={stage} />}
        </For>
      </div>
      <Show when={activeCount() > 0}>
        <span class="ml-auto shrink-0 font-mono text-[9px] text-[var(--color-primary)]">
          {activeCount()} active
        </span>
      </Show>
    </div>
  );
}

function StageTile(props: { stage: PipelineStage }) {
  const meta = STAGE_META[props.stage];
  const entry = () => pipelineStore.stages[props.stage];
  const Icon = meta.icon;

  const tone = () => {
    switch (entry().state) {
      case "started":
      case "progress":
        return "active";
      case "done":
        return "done";
      case "skipped":
        return "skipped";
      default:
        return "idle";
    }
  };

  const fraction = () => {
    const e = entry();
    if (e.total != null && e.count != null && e.total > 0) {
      return Math.min(100, Math.round((e.count / e.total) * 100));
    }
    return null;
  };

  // Compose a tooltip line with whatever signals are useful right now.
  const tooltip = () => {
    const e = entry();
    const parts = [meta.label, `· ${e.state}`];
    if (e.message) parts.push(`· ${e.message}`);
    if (e.count != null && e.total != null) parts.push(`· ${e.count}/${e.total}`);
    return parts.join(" ");
  };

  return (
    <div
      class="surface-raised-subtle flex min-w-[68px] items-center gap-1.5 px-2 py-1"
      classList={{
        "animate-pulse-soft": tone() === "active",
        "opacity-50": tone() === "skipped",
      }}
      style={{
        "background-color":
          tone() === "active"
            ? "color-mix(in oklch, var(--color-primary) 12%, var(--color-surface))"
            : tone() === "done"
              ? "color-mix(in oklch, var(--color-success) 8%, var(--color-surface))"
              : undefined,
      }}
      title={tooltip()}
    >
      <Show
        when={tone() === "active"}
        fallback={
          <Show
            when={tone() === "done"}
            fallback={
              <Show
                when={tone() === "skipped"}
                fallback={<Icon size={10} class="text-[var(--color-foreground-muted)]" />}
              >
                <Minus size={10} class="text-[var(--color-foreground-muted)]" />
              </Show>
            }
          >
            <Check size={10} style={{ color: "var(--color-success)" }} />
          </Show>
        }
      >
        <Loader2 size={10} class="animate-spin" style={{ color: "var(--color-primary)" }} />
      </Show>
      <span
        class="text-[10px] font-medium leading-none"
        style={{
          color:
            tone() === "active"
              ? "var(--color-primary)"
              : tone() === "skipped"
                ? "var(--color-foreground-muted)"
                : undefined,
        }}
      >
        {meta.label}
      </span>
      <Show when={fraction() != null}>
        <span class="font-mono text-[9px] text-[var(--color-foreground-muted)]">
          {fraction()}%
        </span>
      </Show>
    </div>
  );
}
