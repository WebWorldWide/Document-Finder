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
      <div class="px-6 py-5 pt-10">
        <h1 class="text-xl font-semibold">Library</h1>
        <p class="mt-0.5 text-sm text-[var(--color-foreground-muted)]">
          Your saved research collections.
        </p>
      </div>

      <div class="flex-1 overflow-y-auto px-6 pb-6 space-y-4">
        {/* Export error */}
        <Show when={exportError()}>
          <div class="surface-raised-sm flex items-start justify-between gap-2 p-3 text-sm text-[var(--color-destructive)]">
            <span>Export failed: {exportError()}</span>
            <button
              onClick={() => setExportError(null)}
              aria-label="Dismiss"
              class="btn-tactile shrink-0 p-1"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        <Show when={loading()}>
          <div class="flex items-center justify-center py-16">
            <Loader2 size={24} class="animate-spin text-[var(--color-foreground-muted)]" />
          </div>
        </Show>

        <Show when={!loading() && error()}>
          <div class="surface-raised-sm space-y-3 p-4 text-sm text-[var(--color-destructive)]">
            <p>{error()}</p>
            <button
              onClick={() => setLoadTick((n) => n + 1)}
              class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
            >
              <RefreshCw size={12} />
              Retry
            </button>
          </div>
        </Show>

        <Show when={!loading() && !error() && libraries().length === 0}>
          <div class="surface-raised flex flex-col items-center justify-center py-16 text-center mx-auto max-w-md">
            <div class="surface-pressed-sm mb-4 p-5" style={{ "border-radius": "9999px" }}>
              <Archive size={28} class="text-[var(--color-foreground-muted)]" />
            </div>
            <p class="text-sm font-semibold">No libraries yet</p>
            <p class="mt-1 text-sm text-[var(--color-foreground-muted)]">
              Run a search to build your first collection.
            </p>
            <button
              onClick={() => uiStore.setView("find")}
              class="btn-tactile mt-4 px-5 py-2 text-sm font-semibold"
              style={{ background: "var(--color-primary)", color: "white" }}
            >
              Go to Discover
            </button>
          </div>
        </Show>

        <Show when={!loading() && libraries().length > 0}>
          <div class="grid grid-cols-1 gap-5 md:grid-cols-2 xl:grid-cols-3">
            <For each={libraries()}>
              {(lib) => {
                const isActive = () => uiStore.activeLibrary?.path === lib.path;
                const isExporting = () => exportingPath() === lib.path;
                return (
                  <div
                    role="button"
                    tabindex="0"
                    class="group p-5 cursor-pointer outline-none transition-all duration-200 hover:translate-y-[-2px]"
                    classList={{
                      "surface-pressed": isActive(),
                      "surface-raised": !isActive(),
                    }}
                    onClick={() => uiStore.setActiveLibrary(lib)}
                    onKeyDown={(e) => handleCardKeyDown(e, lib)}
                  >
                    <h3
                      class="mb-1 truncate text-sm font-semibold"
                      title={lib.query}
                      style={{ color: isActive() ? "var(--color-primary)" : undefined }}
                    >
                      {lib.query ?? lib.name}
                    </h3>
                    <p class="mb-4 text-xs text-[var(--color-foreground-muted)]">
                      {lib.n_docs} documents · {formatBytes(lib.size_bytes)}
                    </p>
                    <div class="flex gap-2" onClick={(e) => e.stopPropagation()}>
                      <button
                        onClick={() => handleExport(lib)}
                        disabled={isExporting()}
                        class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
                      >
                        <Show when={isExporting()} fallback={<Archive size={12} />}>
                          <Loader2 size={12} class="animate-spin" />
                        </Show>
                        Export ZIP
                      </button>
                      <button
                        onClick={() => api.revealInFinder(lib.path)}
                        class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
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
