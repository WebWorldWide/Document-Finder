import { Show, createMemo, Switch, Match } from "solid-js";
import { Download, CheckCircle2, XCircle, Loader2, Trash2, X, RefreshCw } from "lucide-solid";
import type { ModelInfo } from "@/lib/tauri";
import { modelsStore } from "@/stores/models";
import { formatBytes, formatDuration } from "@/lib/utils";

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

  const eta = createMemo(() => {
    const s = m().status;
    const bps = modelsStore.state.bytesPerSec[m().id] ?? 0;
    if (s.kind !== "downloading" || s.total === 0 || bps === 0) return "";
    const remainingBytes = Math.max(0, s.total - s.downloaded);
    const remainingMs = (remainingBytes / bps) * 1000;
    return `${formatDuration(remainingMs)} remaining`;
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
    <div class="surface-raised-subtle p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <h3 class="truncate text-sm font-semibold">{m().display_name}</h3>
            <span
              class="rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase"
              classList={{
                "bg-[var(--color-primary)]/12 text-[var(--color-primary)]":
                  m().kind === "embedding",
                "bg-amber-500/12 text-[color:var(--warn-ink)]": m().kind === "llm",
              }}
            >
              {m().kind}
            </span>
            <Show when={m().is_default}>
              <span class="rounded-full bg-[var(--color-foreground)]/6 px-1.5 py-0.5 text-[9px] font-medium text-[var(--color-foreground-muted)]">
                default
              </span>
            </Show>
            <Show when={m().license}>
              <span
                class="rounded-full bg-[var(--color-foreground)]/6 px-1.5 py-0.5 text-[9px] font-medium text-[var(--color-foreground-muted)]"
                title="Model weights license"
              >
                {m().license}
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
            {m().description}
          </p>
        </div>

        {/* Prominent percentage pill while downloading */}
        <Show when={m().status.kind === "downloading"}>
          <div
            class="surface-pressed-sm shrink-0 px-3 py-1.5 text-center font-mono text-sm font-bold tabular-nums"
            style={{ color: "var(--color-primary)" }}
          >
            {pct()}%
          </div>
        </Show>

        {/* Action buttons (delete-only when ready; primary path is below for download) */}
        <Show when={m().status.kind === "ready"}>
          <button
            onClick={() => modelsStore.remove(m().id)}
            title="Delete from disk"
            class="btn-tactile flex shrink-0 items-center gap-1 px-2 py-1 text-[11px] font-medium text-[var(--color-foreground-muted)]"
          >
            <Trash2 size={11} />
          </button>
        </Show>
      </div>

      {/* Always-visible status block — never looks "frozen" because we show
       * "Starting…" until bytes/sec arrives. */}
      <div class="mt-3 flex items-center gap-2 text-[10px] text-[var(--color-foreground-muted)]">
        <StatusIcon status={m().status} />
        <span class="font-mono">{sizeLine()}</span>
        <Show when={m().status.kind === "downloading"}>
          <span>·</span>
          <Show when={speed()} fallback={<span class="italic">Starting…</span>}>
            <span class="font-mono">{speed()}</span>
            <Show when={eta()}>
              <span>·</span>
              <span>{eta()}</span>
            </Show>
          </Show>
        </Show>
      </div>

      <Show when={m().status.kind === "downloading"}>
        <div class="progress-capsule-track mt-2 h-2.5 w-full overflow-hidden">
          <div
            class="progress-capsule-fill h-full transition-all duration-300"
            style={{ width: `${pct()}%` }}
          />
        </div>
      </Show>

      {/* Failed: show error + retry button */}
      <Show when={m().status.kind === "failed"}>
        {(_) => {
          const failedStatus = m().status as { kind: "failed"; msg: string };
          return (
            <div class="mt-3 space-y-2">
              <p class="text-[10px] text-[var(--color-destructive)]">{failedStatus.msg}</p>
              <button
                onClick={() => modelsStore.download(m().id)}
                class="btn-tactile flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium"
                style={{ color: "var(--color-primary)" }}
              >
                <RefreshCw size={11} />
                Retry (resumes from partial)
              </button>
            </div>
          );
        }}
      </Show>

      {/* Primary action when not downloaded yet */}
      <Show when={m().status.kind === "not_downloaded" || m().status.kind === "cancelled"}>
        <button
          onClick={() => modelsStore.download(m().id)}
          class="btn-tactile mt-3 flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-semibold"
          style={{ background: "var(--color-primary)", color: "var(--accent-fg)" }}
        >
          <Download size={12} />
          Download {formatBytes(m().approx_bytes)}
        </button>
      </Show>

      {/* Cancel while downloading */}
      <Show when={m().status.kind === "downloading" || m().status.kind === "verifying"}>
        <button
          onClick={() => modelsStore.cancel(m().id)}
          class="btn-tactile mt-3 flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium"
        >
          <X size={11} />
          Cancel
        </button>
      </Show>
    </div>
  );
}

function StatusIcon(props: { status: ModelInfo["status"] }) {
  return (
    <Switch fallback={<Download size={11} style={{ color: "var(--color-foreground-muted)" }} />}>
      <Match when={props.status.kind === "ready"}>
        <CheckCircle2 size={11} style={{ color: "var(--color-success)" }} />
      </Match>
      <Match when={props.status.kind === "downloading" || props.status.kind === "verifying"}>
        <Loader2 size={11} class="animate-spin" />
      </Match>
      <Match when={props.status.kind === "failed"}>
        <XCircle size={11} style={{ color: "var(--color-destructive)" }} />
      </Match>
      <Match when={props.status.kind === "cancelled"}>
        <X size={11} style={{ color: "var(--color-foreground-muted)" }} />
      </Match>
    </Switch>
  );
}
