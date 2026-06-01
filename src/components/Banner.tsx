import { Show, type JSX } from "solid-js";
import { Check, TriangleAlert, X } from "lucide-solid";

/** Dismissible status banner: ok (green) / warn (amber) / bad (red). */
export default function Banner(props: {
  kind?: "ok" | "warn" | "bad";
  children: JSX.Element;
  onDismiss?: () => void;
}) {
  const kind = () => props.kind ?? "ok";
  return (
    <div class={`df-banner ${kind()} fade-in`} role="status">
      <Show when={kind() === "ok"} fallback={<TriangleAlert size={15} />}>
        <Check size={15} />
      </Show>
      <div class="df-banner-body">{props.children}</div>
      <Show when={props.onDismiss}>
        <button class="df-banner-x" onClick={() => props.onDismiss?.()} aria-label="Dismiss">
          <X size={13} />
        </button>
      </Show>
    </div>
  );
}
