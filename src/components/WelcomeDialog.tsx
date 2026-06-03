import { Show, For, onMount, createSignal } from "solid-js";
import { X, CheckCircle2, Brain } from "lucide-solid";
import { modelsStore } from "@/stores/models";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import ModelDownloadCard from "./ModelDownloadCard";
import Logo from "./Logo";
import { formatBytes } from "@/lib/utils";

/// One-time welcome experience. Two passive panels: built-in meta-search
/// (already on, shown for confidence) and the optional AI model downloads.
/// Surfaces once on first launch (or after wiping app state).
export default function WelcomeDialog() {
  const [modelsOpen, setModelsOpen] = createSignal(true);
  const headingId = "welcome-dialog-title";
  let dialogRef: HTMLDivElement | undefined;
  let previouslyFocused: HTMLElement | null = null;

  onMount(() => {
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  const open = () =>
    !settings.aiOnboardingDismissed &&
    !modelsStore.embeddingReady &&
    !modelsStore.llmReady &&
    modelsStore.state.models.length > 0;

  const totalModelSize = () =>
    modelsStore.state.models
      .filter((m) => m.is_default)
      .reduce((acc, m) => acc + m.approx_bytes, 0);

  function dismiss() {
    setSettings("aiOnboardingDismissed", true);
    saveSettings();
    // Restore focus to whatever the user was on before the modal opened.
    previouslyFocused?.focus?.();
  }

  // When the dialog mounts, remember the prior focus and move focus into it so
  // screen readers announce the modal and the Esc / focus-trap handlers fire.
  function captureDialog(el: HTMLDivElement) {
    dialogRef = el;
    previouslyFocused = document.activeElement as HTMLElement | null;
    queueMicrotask(() => el.focus());
  }

  // Esc closes; Tab is trapped so focus can't escape behind the modal.
  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      dismiss();
      return;
    }
    if (e.key !== "Tab" || !dialogRef) return;
    const focusable = Array.from(
      dialogRef.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input, select, textarea, [tabindex]:not([tabindex="-1"])',
      ),
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function downloadDefaults() {
    const defaults = modelsStore.state.models.filter((m) => m.is_default);
    for (const m of defaults) {
      void modelsStore.download(m.id);
    }
  }

  return (
    <Show when={open()}>
      <div class="animate-fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6 backdrop-blur-sm">
        <div
          ref={captureDialog}
          role="dialog"
          aria-modal="true"
          aria-labelledby={headingId}
          tabindex="-1"
          onKeyDown={handleKeyDown}
          class="material-linen border-stitched relative max-h-[88vh] w-full max-w-xl overflow-y-auto p-6 outline-none"
          style={{ "box-shadow": "0 24px 60px rgba(0,0,0,0.25), 0 0 0 0.5px var(--line)" }}
        >
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            class="btn-tactile absolute top-3 right-3 p-1.5 text-[var(--color-foreground-muted)]"
          >
            <X size={14} />
          </button>

          <div class="mb-2 flex items-center gap-2">
            <Logo size={32} style={{ "border-radius": "8px" }} />
            <h2 id={headingId} class="text-embossed text-base font-semibold">
              Welcome to Document Finder
            </h2>
          </div>
          <p class="mb-5 text-xs leading-relaxed text-[var(--color-muted-foreground)]">
            Three optional setup steps. Each is independent — skip whatever you don't need.
          </p>

          {/* Section 1: Built-in meta-search — passive, just showing it's on */}
          <section class="surface-raised-subtle mb-3 p-4">
            <div class="flex items-start gap-3">
              <CheckCircle2
                size={18}
                class="mt-0.5 shrink-0"
                style={{ color: "var(--color-success)" }}
              />
              <div class="flex-1">
                <p class="text-sm font-semibold">Built-in web search is active</p>
                <p class="mt-0.5 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
                  Six engines (DuckDuckGo, Brave, Bing, Mojeek, Marginalia, Startpage) are queried
                  in parallel and deduped. No setup required — searches work immediately.
                </p>
              </div>
            </div>
          </section>

          {/* Section 2: AI Models (default-expanded since this is the big win) */}
          <section class="surface-raised-subtle mb-4 p-4">
            <button
              onClick={() => setModelsOpen((v) => !v)}
              aria-expanded={modelsOpen()}
              class="flex w-full items-start gap-3 text-left"
            >
              <Brain size={18} class="mt-0.5 shrink-0" style={{ color: "var(--color-primary)" }} />
              <div class="flex-1">
                <p class="text-sm font-semibold">
                  AI models (~{formatBytes(totalModelSize())} total)
                </p>
                <p class="mt-0.5 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
                  Two local models power semantic reranking and LLM query expansion + borderline
                  filtering.
                </p>
              </div>
              <span class="ml-auto shrink-0 text-[10px] text-[var(--color-foreground-muted)]">
                {modelsOpen() ? "−" : "+"}
              </span>
            </button>
            <Show when={modelsOpen()}>
              <div class="mt-3 space-y-2 border-t border-[var(--color-border)] pt-3">
                <For each={modelsStore.state.models.filter((m) => m.is_default)}>
                  {(model) => <ModelDownloadCard model={model} />}
                </For>
                <button
                  onClick={() => void downloadDefaults()}
                  class="btn-tactile mt-2 w-full px-3 py-2 text-xs font-semibold"
                  style={{ background: "var(--color-primary)", color: "white" }}
                >
                  Download both
                </button>
              </div>
            </Show>
          </section>

          {/* Footer — single dismiss action */}
          <div class="mt-2 flex items-center justify-between gap-3">
            <p class="text-[10px] text-[var(--color-muted-foreground)]">
              You can manage everything later in Settings.
            </p>
            <button onClick={dismiss} class="btn-tactile px-4 py-2 text-xs font-semibold">
              Done
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
