import { Show, createEffect, onCleanup } from "solid-js";
import { uiStore } from "@/stores/ui";

/** Global confirm dialog + toast, driven by uiStore. */
export default function Overlays() {
  let confirmBtn: HTMLButtonElement | undefined;
  let lastFocus: Element | null = null;

  createEffect(() => {
    const req = uiStore.confirmRequest;
    if (!req) return;
    lastFocus = document.activeElement;
    queueMicrotask(() => confirmBtn?.focus());
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") req.resolve(false);
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      (lastFocus as HTMLElement | null)?.focus?.();
    });
  });

  return (
    <>
      <Show when={uiStore.confirmRequest}>
        {(req) => (
          <div
            class="df-overlay"
            onClick={(e) => e.target === e.currentTarget && req().resolve(false)}
          >
            <div
              class="df-dialog"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="df-dialog-title"
              aria-describedby="df-dialog-body"
            >
              <h2 id="df-dialog-title" class="df-title">
                {req().title}
              </h2>
              <p id="df-dialog-body" class="df-muted" style={{ margin: 0 }}>
                {req().body}
              </p>
              <div class="df-dialog-actions">
                <button class="df-btn df-btn-quiet" onClick={() => req().resolve(false)}>
                  Cancel
                </button>
                <button
                  ref={confirmBtn}
                  class="df-btn"
                  classList={{ "df-btn-danger": !!req().danger, "df-btn-primary": !req().danger }}
                  onClick={() => req().resolve(true)}
                >
                  {req().confirmLabel}
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>
      <Show when={uiStore.toast}>
        {(msg) => (
          <div class="df-toast" aria-hidden="true">
            {msg()}
          </div>
        )}
      </Show>
    </>
  );
}
