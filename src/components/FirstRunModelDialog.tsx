import { Show, For, onMount } from "solid-js";
import { Sparkles, X } from "lucide-solid";
import { modelsStore } from "@/stores/models";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import ModelDownloadCard from "./ModelDownloadCard";
import { formatBytes } from "@/lib/utils";

/// Surfaces once on first launch (or after a fresh wipe of model state).
/// Offers to download both default models so the user gets the full
/// AI-enhanced search experience without having to discover Settings.
export default function FirstRunModelDialog() {
  onMount(() => {
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  const open = () =>
    !settings.aiOnboardingDismissed &&
    !modelsStore.embeddingReady &&
    !modelsStore.llmReady &&
    modelsStore.state.models.length > 0;

  const totalSize = () =>
    modelsStore.state.models
      .filter((m) => m.is_default)
      .reduce((acc, m) => acc + m.approx_bytes, 0);

  function dismiss() {
    setSettings("aiOnboardingDismissed", true);
    saveSettings();
  }

  async function downloadDefaults() {
    const defaults = modelsStore.state.models.filter((m) => m.is_default);
    for (const m of defaults) {
      void modelsStore.download(m.id);
    }
  }

  return (
    <Show when={open()}>
      <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6 backdrop-blur-sm animate-fade-in">
        <div class="relative w-full max-w-lg rounded-2xl border border-[var(--color-border)] bg-[var(--color-card)] p-6 shadow-2xl">
          <button
            onClick={dismiss}
            aria-label="Dismiss"
            class="absolute right-3 top-3 rounded-md p-1 text-[var(--color-muted-foreground)] hover:bg-[var(--color-accent)]"
          >
            <X size={14} />
          </button>

          <div class="mb-4 flex items-center gap-2">
            <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--color-primary)] text-white">
              <Sparkles size={16} />
            </div>
            <h2 class="text-base font-semibold">Enable smarter search</h2>
          </div>

          <p class="mb-4 text-xs leading-relaxed text-[var(--color-muted-foreground)]">
            Document Finder can run two small AI models locally — entirely
            offline, no API keys — to dramatically improve search quality.
            They handle semantic reranking and natural-language query
            expansion. Total download:{" "}
            <strong class="text-[var(--color-foreground)]">
              ~{formatBytes(totalSize())}
            </strong>
            .
          </p>

          <div class="space-y-2">
            <For each={modelsStore.state.models.filter((m) => m.is_default)}>
              {(model) => <ModelDownloadCard model={model} />}
            </For>
          </div>

          <div class="mt-5 flex items-center justify-between gap-3">
            <button
              onClick={dismiss}
              class="text-[11px] text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
            >
              Skip — use lexical ranking only
            </button>
            <button
              onClick={() => {
                void downloadDefaults();
                dismiss();
              }}
              class="rounded-lg bg-[var(--color-primary)] px-4 py-2 text-xs font-medium text-white hover:opacity-90"
            >
              Download both
            </button>
          </div>

          <p class="mt-3 text-[10px] text-[var(--color-muted-foreground)]">
            You can manage models any time from Settings → AI Models.
          </p>
        </div>
      </div>
    </Show>
  );
}
