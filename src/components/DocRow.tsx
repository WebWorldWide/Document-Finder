import { Show } from "solid-js";
import { Check } from "lucide-solid";
import { SOURCE_LABELS, sourceColor, formatBytes, type FileType } from "@/lib/utils";

export interface StreamDoc {
  source: string;
  title: string;
  ftype?: FileType | null;
  // in-flight
  downloaded?: number;
  total?: number;
  // saved / failed
  bytes?: number;
  error?: string;
  status?: "done" | "failed";
}

/** A single document row used in the live stream and the library detail. */
export default function DocRow(props: { doc: StreamDoc; kind: "in-flight" | "saved" }) {
  const color = () => sourceColor(props.doc.source);
  const label = () => SOURCE_LABELS[props.doc.source] ?? props.doc.source;
  const pct = () => {
    const t = props.doc.total ?? 0;
    return t > 0 ? Math.round(((props.doc.downloaded ?? 0) / t) * 100) : null;
  };
  return (
    <div
      classList={{ "df-doc": true, "in-flight": props.kind === "in-flight" }}
      style={{ "--src-color": color() }}
    >
      <span class="df-doc-source">{label()}</span>
      <div class="df-doc-main">
        <span class="df-doc-title" title={props.doc.title}>
          <Show when={props.doc.ftype}>
            <span class={`df-ftype ${props.doc.ftype}`} style={{ "margin-right": "6px" }}>
              {props.doc.ftype}
            </span>
          </Show>
          {props.doc.title}
        </span>
        <Show when={props.kind === "in-flight"}>
          <div class="df-doc-progress">
            <div class="df-progress-track">
              <div
                class="df-progress-fill"
                style={{ width: pct() == null ? "30%" : `${pct()}%`, background: color() }}
              />
            </div>
            <span class="df-doc-bytes">
              {pct() == null
                ? `${formatBytes(props.doc.downloaded ?? 0)} streaming…`
                : `${formatBytes(props.doc.downloaded ?? 0)} / ${formatBytes(props.doc.total ?? 0)}`}
            </span>
          </div>
        </Show>
        <Show when={props.kind === "saved" && props.doc.error}>
          <span class="df-doc-bytes" style={{ color: "var(--bad)" }} title={props.doc.error}>
            {props.doc.error}
          </span>
        </Show>
      </div>
      <Show when={props.kind === "saved"}>
        <span
          class={`df-doc-status ${
            props.doc.status === "done" ? "ok" : props.doc.status === "failed" ? "bad" : ""
          }`}
        >
          <Show when={props.doc.status === "done"} fallback={<span>failed</span>}>
            <Check size={11} />
            <Show when={props.doc.bytes}>{formatBytes(props.doc.bytes ?? 0)}</Show>
          </Show>
        </span>
      </Show>
    </div>
  );
}
