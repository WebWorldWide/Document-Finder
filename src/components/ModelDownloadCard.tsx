import { Show, createMemo } from "solid-js";
import { Download, CheckCircle2, XCircle, Loader2, Trash2, X } from "lucide-solid";
import type { ModelInfo } from "@/lib/tauri";
import { modelsStore } from "@/stores/models";
import { formatBytes } from "@/lib/utils";

export default function ModelDownloadCard(props: { model: ModelInfo }) {
  const m = () => props.model;

  const pct = createMemo(() => {
    const s = m().status;
    if (s.kind !== "downloading" || s.total === 0) return 0;
    return Math.min(100, Math.round((s.downloaded / s.total) * 100));
  });

  const speed = createMemo(() => {
    const bps = modelsStore.state.bytesPerSec[m().id] ?? 0;
    return bps > 0 ? `${formatBytes(bps)}/s` : "";
  });

  const sizeLine = createMemo(() => {
    const s = m().status;
    if (s.kind === "downloading") {
      return `${formatBytes(s.downloaded)} / ${formatBytes(s.total || m().approx_bytes)}`;
    }
    if (s.kind === "ready") {
      return formatBytes(m().on_disk_bytes || m().approx_bytes);
    }
    return `~${formatBytes(m().approx_bytes)}`;
  });

  return (
    <div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <h3 class="truncate text-sm font-semibold">{m().display_name}</h3>
            <span
              class="rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase"
              classList={{
                "bg-[var(--color-primary)]/10 text-[var(--color-primary)]": m().kind === "embedding",
                "bg-amber-500/10 text-amber-600": m().kind === "llm",
              }}
            >
              {m().kind}
            </span>
            <Show when={m().is_default}>
              <span class="rounded-full bg-[var(--color-muted)] px-1.5 py-0.5 text-[9px] font-medium text-[var(--color-muted-foreground)]">
                default
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] leading-relaxed text-[var(--color-muted-foreground)]">
            {m().description}
          </p>

          <div class="mt-2 flex items-center gap-2 text-[10px] text-[var(--color-muted-foreground)]">
            <StatusIcon status={m().status} />
            <span class="font-mono">{sizeLine()}</span>
            <Show when={m().status.kind === "downloading" && speed()}>
              <span>·</span>
              <span class="font-mono">{speed()}</span>
            </Show>
          </div>

          <Show when={m().status.kind === "downloading"}>
            <div class="mt-2 h-1 w-full overflow-hidden rounded-full bg-[var(--color-border)]">
              <div
                class="h-full bg-[var(--color-primary)] transition-all duration-300"
                style={{ width: `${pct()}%` }}
              />
            </div>
          </Show>

          <Show when={m().status.kind === "failed"}>
            {(_) => {
              const failedStatus = m().status as { kind: "failed"; msg: string };
              return (
                <p class="mt-2 text-[10px] text-[var(--color-destructive)]">
                  {failedStatus.msg}
                </p>
              );
            }}
          </Show>
        </div>

        <div class="shrink-0 flex items-center gap-1.5">
          <Show when={m().status.kind === "not_downloaded" || m().status.kind === "failed" || m().status.kind === "cancelled"}>
            <button
              onClick={() => modelsStore.download(m().id)}
              class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-[11px] font-medium hover:bg-[var(--color-accent)] transition-colors"
            >
              <Download size={11} />
              Download
            </button>
          </Show>
          <Show when={m().status.kind === "downloading" || m().status.kind === "verifying"}>
            <button
              onClick={() => modelsStore.cancel(m().id)}
              class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-[11px] font-medium hover:bg-[var(--color-accent)] transition-colors"
            >
              <X size={11} />
              Cancel
            </button>
          </Show>
          <Show when={m().status.kind === "ready"}>
            <button
              onClick={() => modelsStore.remove(m().id)}
              title="Delete from disk"
              class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2 py-1 text-[11px] font-medium text-[var(--color-muted-foreground)] hover:bg-[var(--color-destructive)]/10 hover:text-[var(--color-destructive)] transition-colors"
            >
              <Trash2 size={11} />
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function StatusIcon(props: { status: ModelInfo["status"] }) {
  switch (props.status.kind) {
    case "ready":
      return <CheckCircle2 size={11} style={{ color: "var(--color-success)" }} />;
    case "downloading":
    case "verifying":
      return <Loader2 size={11} class="animate-spin" />;
    case "failed":
      return <XCircle size={11} style={{ color: "var(--color-destructive)" }} />;
    case "cancelled":
      return <X size={11} style={{ color: "var(--color-muted-foreground)" }} />;
    default:
      return <Download size={11} style={{ color: "var(--color-muted-foreground)" }} />;
  }
}
