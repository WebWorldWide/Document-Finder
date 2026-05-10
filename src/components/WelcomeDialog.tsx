import { Show, For, onMount, createSignal } from "solid-js";
import { Sparkles, X, CheckCircle2, Server, Brain } from "lucide-solid";
import { modelsStore } from "@/stores/models";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import ModelDownloadCard from "./ModelDownloadCard";
import SearxngSetupPanel from "./SearxngSetupPanel";
import { formatBytes } from "@/lib/utils";

/// One-time welcome experience covering the three things a fresh user
/// might want to opt into: built-in meta-search (already on, just shown
/// for confidence), SearXNG via Docker (optional power-user backend),
/// and the local AI models. Replaces the previous models-only
/// FirstRunModelDialog.
///
/// Surfaces once on first launch (or after wiping app state). Each
/// section is independent — the user can set up any combination.
export default function WelcomeDialog() {
  // Local UI state for which sections the user has explicitly expanded.
  // Keep them collapsed by default so the dialog opens compact.
  const [searxngOpen, setSearxngOpen] = createSignal(false);
  const [modelsOpen, setModelsOpen] = createSignal(true);

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
  }

  function downloadDefaults() {
    const defaults = modelsStore.state.models.filter((m) => m.is_default);
    for (const m of defaults) {
      void modelsStore.download(m.id);
    }
  }

  return (
    <Show when={open()}>
      <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6 backdrop-blur-sm animate-fade-in">
        <div class="material-linen border-stitched relative w-full max-w-xl max-h-[88vh] overflow-y-auto p-6"
          style={{ "box-shadow": "var(--shadow-floating), inset 0 1px 0 oklch(1 0 0 / 0.95)" }}
        >
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            class="btn-tactile absolute right-3 top-3 p-1.5 text-[var(--color-foreground-muted)]"
          >
            <X size={14} />
          </button>

          <div class="mb-2 flex items-center gap-2">
            <div
              class="surface-glossy flex h-8 w-8 items-center justify-center rounded-lg text-white"
              style={{
                background:
                  "linear-gradient(135deg, var(--color-accent-warm) 0%, var(--color-primary) 60%, var(--color-accent-cool) 100%)",
                "box-shadow": "var(--shadow-raised-xs), inset 0 1px 0 oklch(1 0 0 / 0.6), inset 0 -1px 0 oklch(0 0 0 / 0.18)",
              }}
            >
              <Sparkles size={16} />
            </div>
            <h2 class="text-base font-semibold text-embossed">Welcome to Document Finder</h2>
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
                  Six engines (DuckDuckGo, Brave, Bing, Mojeek, Marginalia, Startpage)
                  are queried in parallel and deduped. No setup required — searches
                  work immediately.
                </p>
              </div>
            </div>
          </section>

          {/* Section 2: SearXNG (optional Docker setup) */}
          <section class="surface-raised-subtle mb-3 p-4">
            <button
              onClick={() => setSearxngOpen((v) => !v)}
              aria-expanded={searxngOpen()}
              class="flex w-full items-start gap-3 text-left"
            >
              <Server size={18} class="mt-0.5 shrink-0 text-[var(--color-foreground-muted)]" />
              <div class="flex-1">
                <p class="text-sm font-semibold">SearXNG (optional)</p>
                <p class="mt-0.5 text-[11px] leading-relaxed text-[var(--color-foreground-muted)]">
                  Add 70+ extra search engines via a local Docker container.
                  Most users don't need this — the built-in set is broad enough.
                </p>
              </div>
              <span class="ml-auto shrink-0 text-[10px] text-[var(--color-foreground-muted)]">
                {searxngOpen() ? "−" : "+"}
              </span>
            </button>
            <Show when={searxngOpen()}>
              <div class="mt-3 border-t border-[var(--color-border)] pt-3">
                <SearxngSetupPanel compact />
              </div>
            </Show>
          </section>

          {/* Section 3: AI Models (default-expanded since this is the big win) */}
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
                  Two local models power semantic reranking + LLM query expansion
                  and borderline filtering. Everything runs offline — no API keys,
                  no telemetry. Recommended.
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
            <button
              onClick={dismiss}
              class="btn-tactile px-4 py-2 text-xs font-semibold"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
