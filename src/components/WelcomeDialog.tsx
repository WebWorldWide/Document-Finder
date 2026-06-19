import { Show, For, onMount, createSignal } from "solid-js";
import { X, CheckCircle2, Brain, Download, AlertCircle, RotateCw, Loader2 } from "lucide-solid";
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

  // Gate ONLY on the explicit one-time dismiss flag (+ the registry having
  // loaded). Previously this ALSO required !embeddingReady && !llmReady, but the
  // default "Balanced" preset auto-caches the BGE embedding on the first search
  // (warm_in_background_implicit), and that cache lives in the bundle-identifier
  // app-data dir — so after a single search `embeddingReady` is permanently true
  // and the welcome modal NEVER appears again, even though the user never saw or
  // dismissed it. The dismiss flag is the real source of truth for "has the user
  // been through onboarding"; tying visibility to a background cache side-effect
  // also made the modal vanish out from under the user mid-download.
  // Show once the model list has finished its first load attempt — NOT gated on
  // it succeeding. If list_models errors (or a non-AI build returns empty), the
  // onboarding must still appear so the user sees the (AI-independent) web-search
  // reassurance, can dismiss it, and the dismiss flag gets persisted — otherwise
  // the app silently re-attempts the welcome every launch with nothing shown.
  // The AI section renders its own empty/error fallback below.
  const open = () =>
    !settings.aiOnboardingDismissed &&
    !modelsStore.state.loading &&
    (modelsStore.state.models.length > 0 || modelsStore.state.error !== null);

  // The BGE embedding isn't in the registry (fastembed-managed), so add its
  // approximate on-disk size to the headline total — otherwise "~total" only
  // counts the LLM and undercounts what the "Download both" action fetches.
  const EMBED_APPROX_BYTES = 66_000_000;
  const totalModelSize = () =>
    modelsStore.state.models
      .filter((m) => m.is_default)
      .reduce((acc, m) => acc + m.approx_bytes, 0) + EMBED_APPROX_BYTES;

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
    // Treat the dialog container (tabindex=-1, where focus starts) as the first
    // boundary too — otherwise a Shift+Tab while focus is still on the container
    // falls through to the background behind the modal.
    if (e.shiftKey && (document.activeElement === first || document.activeElement === dialogRef)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // Pull focus back into the dialog if it escapes — e.g. the focused control was
  // reactively unmounted (the Download button after it's clicked), dropping focus
  // to <body> outside the trap. Ignore moves to another element inside the dialog
  // and window-blur (relatedTarget on another app), which we don't want to fight.
  function handleFocusOut(e: FocusEvent) {
    const next = e.relatedTarget as Node | null;
    if (!dialogRef) return;
    if (next === null || !dialogRef.contains(next)) {
      // null relatedTarget from an unmount → refocus; but only while the document
      // still has focus (don't steal it back when the user switched apps).
      if (document.hasFocus()) queueMicrotask(() => dialogRef?.focus());
    }
  }

  // Embedding model is fastembed-managed (no registry card); it auto-downloads
  // on first use and these flags reflect that.
  // A cached-but-failed-to-load embedding keeps embeddingDownloaded=true, so
  // guard on the error too — otherwise the green "AI models ready" footer shows
  // at the same time as the card's red "Couldn't load", and downloadDefaults()
  // skips re-warming the errored model.
  const embeddingReady = () =>
    !modelsStore.state.embeddingError &&
    !modelsStore.state.embeddingWarming &&
    (modelsStore.state.embeddingLoaded || modelsStore.state.embeddingDownloaded);
  // The registry list actually loaded a default model (not the error/empty
  // state). Guards "ready"/size claims so we never assert all-ready or a precise
  // total when list_models failed and the LLM genuinely isn't installed.
  const hasDefaultModel = () =>
    modelsStore.state.error === null && modelsStore.state.models.some((m) => m.is_default);
  // Default LLMs (the only registry downloads) that still need fetching.
  const llmsToDownload = () =>
    modelsStore.state.models.filter((m) => m.is_default && m.status.kind !== "ready");
  // Something is mid-fetch (a default LLM downloading/verifying, or the embedding
  // warming) — the per-model card / embedding row shows its own progress, so the
  // aggregate "Download" button hides to avoid a second click re-issuing the
  // in-flight download.
  const anyLlmBusy = () =>
    modelsStore.state.embeddingWarming ||
    modelsStore.state.models.some(
      (m) => m.is_default && (m.status.kind === "downloading" || m.status.kind === "verifying"),
    );
  // How many distinct things the aggregate button would still fetch (missing
  // LLMs + the embedding if it isn't ready) — drives an accurate label.
  const toDownloadCount = () => llmsToDownload().length + (embeddingReady() ? 0 : 1);
  // Everything the AI features need is already present. Requires the LLM list to
  // have actually loaded a default — otherwise a failed list (llmsToDownload
  // filters an empty array → length 0) would falsely report "AI models ready".
  const allReady = () => hasDefaultModel() && llmsToDownload().length === 0 && embeddingReady();

  function downloadDefaults() {
    // Only fetch what's actually missing — re-issuing a download for an
    // already-installed model just flickers the UI and looks broken. Warm the
    // embedding only if it isn't already loaded/cached.
    for (const m of llmsToDownload()) {
      void modelsStore.download(m.id);
    }
    if (!embeddingReady()) void modelsStore.warmEmbedding();
  }

  return (
    <Show when={open()}>
      <div class="fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6 backdrop-blur-sm">
        <div
          ref={captureDialog}
          role="dialog"
          aria-modal="true"
          aria-labelledby={headingId}
          tabindex="-1"
          onKeyDown={handleKeyDown}
          onFocusOut={handleFocusOut}
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
            Everything below is optional and independent — skip whatever you don't need. Searches
            work right away.
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
                  AI models
                  <Show when={hasDefaultModel()}> (~{formatBytes(totalModelSize())} total)</Show>
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
                {/* Embedding model — fastembed-managed, not a registry download
                    card. Rendered as a FIRST-CLASS card with the same visual
                    weight as the LLM card below, so "two models" / "Download both"
                    reads clearly. It has no Download button because it fetches +
                    warms automatically (often before this dialog is even read). */}
                <div class="surface-raised-subtle p-4">
                  <div class="flex items-center gap-2">
                    <h3 class="truncate text-sm font-semibold">
                      Semantic-search model (BGE-Small)
                    </h3>
                    <span class="rounded-full bg-[var(--color-primary)]/12 px-1.5 py-0.5 text-[9px] font-medium text-[var(--accent-ink)] uppercase">
                      embedding
                    </span>
                    <span class="rounded-full bg-[var(--color-foreground)]/6 px-1.5 py-0.5 text-[9px] font-medium text-[var(--color-foreground-muted)]">
                      default
                    </span>
                    <span
                      class="rounded-full bg-[var(--color-foreground)]/6 px-1.5 py-0.5 text-[9px] font-medium text-[var(--color-foreground-muted)]"
                      title="Model weights license"
                    >
                      MIT
                    </span>
                  </div>
                  <p class="mt-1 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
                    Powers semantic reranking so the most relevant results rise to the top. Managed
                    automatically — no manual download.
                  </p>
                  <div class="mt-3 flex items-center gap-2 text-[10px] text-[var(--color-foreground-muted)]">
                    <Show
                      when={!modelsStore.state.embeddingError}
                      fallback={
                        <>
                          <AlertCircle
                            size={11}
                            class="shrink-0"
                            style={{ color: "var(--color-destructive)" }}
                          />
                          <span style={{ color: "var(--color-destructive)" }}>
                            Couldn&rsquo;t load (see Settings)
                          </span>
                          <button
                            onClick={() => void modelsStore.warmEmbedding()}
                            class="btn-tactile ml-1 flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-medium"
                            style={{ color: "var(--accent-ink)" }}
                          >
                            <RotateCw size={10} /> Retry
                          </button>
                        </>
                      }
                    >
                      {/* Warming is checked FIRST: embeddingDownloaded is sticky
                          (stays true after a load failure), so on a cached-but-
                          failed Retry it would otherwise show "Ready" while the
                          worker is still re-loading. */}
                      <Show
                        when={modelsStore.state.embeddingWarming}
                        fallback={
                          <Show
                            when={
                              modelsStore.state.embeddingLoaded ||
                              modelsStore.state.embeddingDownloaded
                            }
                            fallback={
                              <>
                                <Download
                                  size={11}
                                  style={{ color: "var(--color-foreground-muted)" }}
                                />
                                <span class="font-mono">~66 MB</span>
                                <span>·</span>
                                <span>downloads automatically on first search</span>
                              </>
                            }
                          >
                            <CheckCircle2 size={11} style={{ color: "var(--color-success)" }} />
                            <span>Ready · managed automatically</span>
                          </Show>
                        }
                      >
                        <Loader2 size={11} class="spin shrink-0" />
                        <span>Downloading…</span>
                      </Show>
                    </Show>
                  </div>
                </div>
                <Show
                  when={modelsStore.state.models.some((m) => m.is_default)}
                  fallback={
                    <p class="surface-raised-subtle p-3 text-[11px] text-[var(--color-foreground-muted)]">
                      Couldn&rsquo;t load the model list right now — you can manage AI models later
                      in Settings.
                    </p>
                  }
                >
                  <For each={modelsStore.state.models.filter((m) => m.is_default)}>
                    {(model) => <ModelDownloadCard model={model} hideDownload />}
                  </For>
                </Show>
                <Show
                  when={!allReady()}
                  fallback={
                    <div class="mt-2 flex items-center justify-center gap-1.5 py-2 text-xs font-semibold">
                      <CheckCircle2 size={14} style={{ color: "var(--color-success)" }} />
                      <span style={{ color: "var(--color-success)" }}>AI models ready</span>
                    </div>
                  }
                >
                  {/* Hidden while a download is already in flight — the per-model
                      card shows progress + cancel, and a second click here would
                      re-issue the in-flight fetch. */}
                  <Show
                    when={!anyLlmBusy()}
                    fallback={
                      <p class="mt-2 py-2 text-center text-[11px] text-[var(--color-foreground-muted)]">
                        Downloading… you can keep using the app.
                      </p>
                    }
                  >
                    <button
                      onClick={() => void downloadDefaults()}
                      class="btn-tactile mt-2 w-full px-3 py-2 text-xs font-semibold"
                      style={{
                        // Darken 12% like .df-btn.accent so the label clears AA
                        // on the light-theme accents (raw sky is ~3.4:1).
                        background: "color-mix(in oklch, var(--color-primary) 88%, black)",
                        color: "var(--accent-fg)",
                      }}
                    >
                      {toDownloadCount() > 1 ? "Download both" : "Download model"}
                    </button>
                  </Show>
                </Show>
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
