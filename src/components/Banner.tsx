import { Show, type JSX } from "solid-js";
import { Check, TriangleAlert, X } from "lucide-solid";

/** Dismissible status banner: ok (green) / warn (amber) / bad (red). */
export default function Banner(props: {
  kind?: "ok" | "warn" | "bad";
  children: JSX.Element;
  onDismiss?: () => void;
}) {
  const kind = () => props.kind ?? "ok";
  // Errors announce assertively (interrupt the screen reader); ok/warn are
  // polite. A polite region added at the same time as its content is often
  // skipped, so genuine error feedback the user triggered would be missed.
  const isError = () => kind() === "bad";
  return (
    <div
      class={`df-banner ${kind()} fade-in`}
      role={isError() ? "alert" : "status"}
      aria-live={isError() ? "assertive" : "polite"}
    >
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
