import { Show, type JSX } from "solid-js";
import { AlertTriangle, CheckCircle2, Info, X, XCircle } from "lucide-solid";

/** Inline status note (replaces the old banner stack). */
export default function Note(props: {
  tone?: "info" | "ok" | "warn" | "bad";
  children: JSX.Element;
  onDismiss?: () => void;
}) {
  const tone = () => props.tone ?? "info";
  const Icon = () => {
    switch (tone()) {
      case "ok":
        return <CheckCircle2 size={16} aria-hidden="true" />;
      case "warn":
        return <AlertTriangle size={16} aria-hidden="true" />;
      case "bad":
        return <XCircle size={16} aria-hidden="true" />;
      default:
        return <Info size={16} aria-hidden="true" />;
    }
  };
  return (
    <div class="df-note" data-tone={tone()} role={tone() === "bad" ? "alert" : "status"}>
      <Icon />
      <div>{props.children}</div>
      <Show when={props.onDismiss}>
        <button class="df-note-x" aria-label="Dismiss" onClick={() => props.onDismiss?.()}>
          <X size={14} />
        </button>
      </Show>
    </div>
  );
}
