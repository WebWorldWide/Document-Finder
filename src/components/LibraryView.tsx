import { createSignal, createEffect, onCleanup, Show, For } from "solid-js";
import { Archive, FolderOpen, Loader2, X, RefreshCw } from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import { api, type LibraryInfo } from "@/lib/tauri";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { formatBytes } from "@/lib/utils";

export default function LibraryView() {
  const [libraries, setLibraries] = createSignal<LibraryInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [exportingPath, setExportingPath] = createSignal<string | null>(null);
  const [exportError, setExportError] = createSignal<string | null>(null);
  const [loadTick, setLoadTick] = createSignal(0);

  createEffect(() => {
    const _tick = loadTick();
    const root = settings.libraryRoot;
    if (!root) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    api.listLibraries(root)
      .then((libs) => {
        if (!cancelled) {
          setLibraries(libs);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
        }
      });

    onCleanup(() => { cancelled = true; });
  });

  async function handleExport(lib: LibraryInfo) {
    setExportError(null);
    const dest = await save({
      defaultPath: `${lib.name}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!dest) return;
    setExportingPath(lib.path);
    try {
      const result = await api.exportLibraryZip(lib.path, dest);
      await api.revealInFinder(result.dest);
    } catch (e) {
      setExportError(String(e));
    } finally {
      setExportingPath(null);
    }
  }

  function handleCardKeyDown(e: KeyboardEvent, lib: LibraryInfo) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      uiStore.setActiveLibrary(lib);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-hidden">
      <div class="border-b border-[var(--color-border)] px-6 py-5 pt-10">
        <h1 class="text-xl font-semibold">Library</h1>
        <p class="mt-0.5 text-sm text-[var(--color-muted-foreground)]">
          Your saved research collections.
        </p>
      </div>

      <div class="flex-1 overflow-y-auto p-6 space-y-4">
        {/* Export error */}
        <Show when={exportError()}>
          <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-3 text-sm text-[var(--color-destructive)] flex items-start justify-between gap-2">
            <span>Export failed: {exportError()}</span>
            <button
              onClick={() => setExportError(null)}
              aria-label="Dismiss"
              class="shrink-0 rounded hover:bg-[var(--color-destructive)]/10 p-0.5 transition-colors"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        <Show when={loading()}>
          <div class="flex items-center justify-center py-16">
            <Loader2 size={24} class="animate-spin text-[var(--color-muted-foreground)]" />
          </div>
        </Show>

        <Show when={!loading() && error()}>
          <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-4 text-sm text-[var(--color-destructive)] space-y-3">
            <p>{error()}</p>
            <button
              onClick={() => setLoadTick((n) => n + 1)}
              class="flex items-center gap-1.5 rounded-lg border border-[var(--color-destructive)]/30 px-3 py-1.5 text-xs font-medium hover:bg-[var(--color-destructive)]/10 transition-colors"
            >
              <RefreshCw size={12} />
              Retry
            </button>
          </div>
        </Show>

        <Show when={!loading() && !error() && libraries().length === 0}>
          <div class="flex flex-col items-center justify-center py-20 text-center">
            <div class="mb-4 rounded-full bg-[var(--color-muted)] p-5">
              <Archive size={28} class="text-[var(--color-muted-foreground)]" />
            </div>
            <p class="text-sm font-medium">No libraries yet</p>
            <p class="mt-1 text-sm text-[var(--color-muted-foreground)]">
              Run a search to build your first collection.
            </p>
            <button
              onClick={() => uiStore.setView("find")}
              class="mt-4 rounded-lg bg-[var(--color-primary)] px-4 py-2 text-sm font-medium text-white hover:opacity-90"
            >
              Go to Discover
            </button>
          </div>
        </Show>

        <Show when={!loading() && libraries().length > 0}>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            <For each={libraries()}>
              {(lib) => {
                const isActive = () => uiStore.activeLibrary?.path === lib.path;
                const isExporting = () => exportingPath() === lib.path;
                return (
                  <div
                    role="button"
                    tabindex="0"
                    class="group rounded-xl border p-4 transition-all cursor-pointer hover:shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-primary)]"
                    classList={{
                      "border-[var(--color-primary)] bg-[var(--color-primary)]/5": isActive(),
                      "border-[var(--color-border)] bg-[var(--color-card)] hover:border-[var(--color-primary)]/50": !isActive(),
                    }}
                    onClick={() => uiStore.setActiveLibrary(lib)}
                    onKeyDown={(e) => handleCardKeyDown(e, lib)}
                  >
                    <h3 class="mb-1 truncate text-sm font-medium" title={lib.query}>
                      {lib.query ?? lib.name}
                    </h3>
                    <p class="mb-4 text-xs text-[var(--color-muted-foreground)]">
                      {lib.n_docs} documents · {formatBytes(lib.size_bytes)}
                    </p>
                    <div class="flex gap-2" onClick={(e) => e.stopPropagation()}>
                      <button
                        onClick={() => handleExport(lib)}
                        disabled={isExporting()}
                        class="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-1.5 text-xs font-medium hover:bg-[var(--color-accent)] transition-colors disabled:opacity-50"
                      >
                        <Show when={isExporting()} fallback={<Archive size={12} />}>
                          <Loader2 size={12} class="animate-spin" />
                        </Show>
                        Export ZIP
                      </button>
                      <button
                        onClick={() => api.revealInFinder(lib.path)}
                        class="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-1.5 text-xs font-medium hover:bg-[var(--color-accent)] transition-colors"
                      >
                        <FolderOpen size={12} />
                        Show
                      </button>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}
