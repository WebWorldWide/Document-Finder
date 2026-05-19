import { For, Show, onMount, createMemo } from "solid-js";
import { Download, Trash2, X, CheckCircle2, Loader2, AlertTriangle } from "lucide-solid";
import { modelsStore } from "@/stores/models";
import { formatBytes } from "@/lib/utils";

/// Per-model row showing the download/ready/failed state and the action
/// button matching the current state (Download / Cancel / Delete).
function ModelRow(props: { id: string }) {
  const m = () => modelsStore.state.models.find((x) => x.id === props.id)!;
  const bps = () => modelsStore.state.bytesPerSec[props.id] ?? 0;
  const activity = () => modelsStore.state.activity[props.id];

  const pct = () => {
    const s = m().status;
    if (s.kind !== "downloading" || s.total === 0) return 0;
    return Math.min(100, Math.round((s.downloaded / s.total) * 100));
  };

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--pad-2)",
        padding: "var(--pad-3) 0",
        "border-bottom": "0.5px solid var(--line)",
      }}
    >
      <div style={{ display: "flex", "align-items": "center", gap: "var(--pad-3)" }}>
        <div style={{ flex: 1, "min-width": 0 }}>
          <div
            style={{
              display: "flex",
              "align-items": "baseline",
              gap: "8px",
              "font-size": "13px",
              "font-weight": 600,
              color: "var(--ink)",
            }}
          >
            <span>{m().display_name}</span>
            <span
              style={{
                "font-family": "var(--font-mono)",
                "font-size": "10px",
                "font-weight": 400,
                color: "var(--ink-3)",
              }}
            >
              {m().id}
            </span>
            <Show when={m().is_default}>
              <span
                style={{
                  "font-size": "9.5px",
                  "text-transform": "uppercase",
                  "letter-spacing": "0.06em",
                  color: "var(--accent)",
                  "font-weight": 600,
                }}
              >
                default
              </span>
            </Show>
          </div>
          <div style={{ "font-size": "11.5px", color: "var(--ink-3)", "line-height": 1.4 }}>
            {m().description}
          </div>
        </div>

        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            "align-items": "flex-end",
            gap: "4px",
            "min-width": "140px",
          }}
        >
          {/* Status badge */}
          <Show when={m().status.kind === "ready"}>
            <span
              style={{
                display: "inline-flex",
                "align-items": "center",
                gap: "4px",
                "font-size": "11px",
                color: "var(--ok)",
                "font-weight": 500,
              }}
            >
              <CheckCircle2 size={12} /> Ready · {formatBytes(m().on_disk_bytes)}
            </span>
          </Show>
          <Show when={m().status.kind === "verifying"}>
            <span
              style={{
                display: "inline-flex",
                "align-items": "center",
                gap: "4px",
                "font-size": "11px",
                color: "var(--ink-3)",
              }}
            >
              <Loader2 size={12} class="spin" /> Verifying…
            </span>
          </Show>
          <Show when={m().status.kind === "failed"}>
            <span
              style={{
                display: "inline-flex",
                "align-items": "center",
                gap: "4px",
                "font-size": "11px",
                color: "var(--bad)",
              }}
              title={(m().status as { kind: "failed"; msg: string }).msg}
            >
              <AlertTriangle size={12} /> Failed
            </span>
          </Show>
          <Show when={m().status.kind === "not_downloaded" || m().status.kind === "cancelled"}>
            <span style={{ "font-size": "11px", color: "var(--ink-3)" }}>
              {formatBytes(m().approx_bytes)} to download
            </span>
          </Show>

          {/* Actions */}
          <div style={{ display: "flex", gap: "6px" }}>
            <Show when={m().status.kind === "downloading"}>
              <button class="df-btn sm danger" onClick={() => modelsStore.cancel(m().id)}>
                <X size={12} /> Cancel
              </button>
            </Show>
            <Show
              when={
                m().status.kind === "not_downloaded" ||
                m().status.kind === "cancelled" ||
                m().status.kind === "failed"
              }
            >
              <button class="df-btn sm" onClick={() => modelsStore.download(m().id)}>
                <Download size={12} /> Download
              </button>
            </Show>
            <Show when={m().status.kind === "ready"}>
              <button class="df-btn sm danger" onClick={() => modelsStore.remove(m().id)}>
                <Trash2 size={12} /> Delete
              </button>
            </Show>
          </div>
        </div>
      </div>

      {/* Download progress bar */}
      <Show when={m().status.kind === "downloading"}>
        <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
          <div class="df-progress-track" style={{ flex: 1 }}>
            <div class="df-progress-fill" style={{ width: `${pct()}%` }} />
          </div>
          <span
            style={{
              "font-family": "var(--font-mono)",
              "font-size": "10.5px",
              color: "var(--ink-3)",
              "white-space": "nowrap",
              "font-variant-numeric": "tabular-nums",
            }}
          >
            {(() => {
              const s = m().status as { kind: "downloading"; downloaded: number; total: number };
              const rate = bps() > 0 ? ` · ${formatBytes(bps())}/s` : "";
              return `${formatBytes(s.downloaded)} / ${formatBytes(s.total)}${rate}`;
            })()}
          </span>
        </div>
      </Show>

      {/* Runtime activity hint (embedding / llm_warming / …) */}
      <Show when={activity()}>
        <span
          style={{
            "font-size": "10.5px",
            color: "var(--accent)",
            "font-family": "var(--font-mono)",
          }}
        >
          {activity()?.status}
          {activity()?.detail ? ` — ${activity()?.detail}` : ""}
        </span>
      </Show>
    </div>
  );
}

/// AI Models section in Settings. Lists the registry of available
/// embedding + LLM models with download / cancel / delete controls.
/// Refreshed on mount; updates live via the df:model_progress and
/// df:model_status event subscriptions wired up in main.tsx.
export default function ModelsPanel() {
  onMount(() => modelsStore.refresh());

  const embeddings = createMemo(() =>
    modelsStore.state.models.filter((m) => m.kind === "embedding"),
  );
  const llms = createMemo(() => modelsStore.state.models.filter((m) => m.kind === "llm"));

  return (
    <section class="df-section">
      <h2>AI Models</h2>
      <p class="hint">
        Optional local models for smarter ranking. <strong>Embeddings</strong> drive semantic
        re-rank of search results; <strong>LLMs</strong> expand queries into sub-queries and filter
        borderline candidates. Both run locally — no API keys, no data leaving the machine.
      </p>

      <Show when={modelsStore.state.loading}>
        <div style={{ padding: "var(--pad-4) 0", color: "var(--ink-3)" }}>
          <Loader2 size={14} class="spin" style={{ "vertical-align": "middle" }} /> Loading
          registry…
        </div>
      </Show>

      <Show when={modelsStore.state.error}>
        <div class="df-banner bad">
          <AlertTriangle size={14} />
          <div class="df-banner-body">
            <strong>Could not load AI models.</strong> {modelsStore.state.error}
          </div>
        </div>
      </Show>

      <Show when={!modelsStore.state.loading && embeddings().length > 0}>
        <div
          style={{
            "font-size": "10px",
            "text-transform": "uppercase",
            "letter-spacing": "0.08em",
            color: "var(--ink-3)",
            "font-weight": 600,
            "margin-top": "var(--pad-3)",
            "margin-bottom": "4px",
          }}
        >
          Embeddings
        </div>
        <For each={embeddings()}>{(m) => <ModelRow id={m.id} />}</For>
      </Show>

      <Show when={!modelsStore.state.loading && llms().length > 0}>
        <div
          style={{
            "font-size": "10px",
            "text-transform": "uppercase",
            "letter-spacing": "0.08em",
            color: "var(--ink-3)",
            "font-weight": 600,
            "margin-top": "var(--pad-4)",
            "margin-bottom": "4px",
          }}
        >
          Language models
        </div>
        <For each={llms()}>{(m) => <ModelRow id={m.id} />}</For>
      </Show>

      <Show
        when={
          !modelsStore.state.loading && !modelsStore.state.error && modelsStore.state.models.length
        }
      >
        <div
          style={{
            "margin-top": "var(--pad-4)",
            "padding-top": "var(--pad-3)",
            "border-top": "0.5px solid var(--line)",
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            "font-size": "11px",
            color: "var(--ink-3)",
          }}
        >
          <span>
            Total on disk:{" "}
            <strong
              style={{
                color: "var(--ink)",
                "font-family": "var(--font-mono)",
              }}
            >
              {formatBytes(modelsStore.totalDiskBytes)}
            </strong>
          </span>
          <button class="df-btn ghost sm" onClick={() => modelsStore.refresh()}>
            Refresh
          </button>
        </div>
      </Show>
    </section>
  );
}
