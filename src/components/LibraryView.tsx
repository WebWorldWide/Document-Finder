import { createSignal, createEffect, onCleanup, Show, For } from "solid-js";
import { Archive, FolderOpen, Loader2, X, RefreshCw, Trash2 } from "lucide-solid";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { api, type LibraryInfo } from "@/lib/tauri";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { formatBytes } from "@/lib/utils";

export default function LibraryView() {
  const [libraries, setLibraries] = createSignal<LibraryInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [exportingPath, setExportingPath] = createSignal<string | null>(null);
  const [deletingPath, setDeletingPath] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
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

    api
      .listLibraries(root)
      .then((libs) => {
        if (!cancelled) {
          setLibraries(libs);
          uiStore.setKnownLibraries(libs);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
        }
      });

    onCleanup(() => {
      cancelled = true;
    });
  });

  async function handleExport(lib: LibraryInfo) {
    setActionError(null);
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
      setActionError(`Export failed: ${String(e)}`);
    } finally {
      setExportingPath(null);
    }
  }

  async function handleDelete(lib: LibraryInfo) {
    setActionError(null);
    const ok = await ask(
      `Delete library "${lib.query ?? lib.name}"? This permanently removes ${lib.n_docs} document${lib.n_docs === 1 ? "" : "s"} and ${formatBytes(lib.size_bytes)} from disk. This cannot be undone.`,
      { title: "Delete Library", kind: "warning" },
    );
    if (!ok) return;
    // If the active library is the one being deleted, clear it first so the
    // SQLite handle held by views drops before the directory disappears.
    if (uiStore.activeLibrary?.path === lib.path) {
      uiStore.setActiveLibrary(null);
    }
    setDeletingPath(lib.path);
    try {
      await api.deleteLibrary(lib.path);
      setLoadTick((n) => n + 1);
    } catch (e) {
      setActionError(`Delete failed: ${String(e)}`);
    } finally {
      setDeletingPath(null);
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
        <h1 class="text-embossed text-xl font-semibold">Library</h1>
        <p class="mt-0.5 text-sm text-[var(--color-foreground-muted)]">
          Your saved research collections.
        </p>
      </div>

      <div class="flex-1 space-y-4 overflow-y-auto px-6 pb-6">
        {/* Action error (export or delete) */}
        <Show when={actionError()}>
          <div class="surface-raised-sm flex items-start justify-between gap-2 p-3 text-sm text-[var(--color-destructive)]">
            <span class="break-words">{actionError()}</span>
            <button
              onClick={() => setActionError(null)}
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
          <div class="material-leather border-stitched-dark mx-auto flex max-w-md flex-col items-center justify-center py-16 text-center">
            <div
              class="surface-pressed-sm mb-4 p-5"
              style={{ "border-radius": "9999px", background: "oklch(0.32 0.05 50)" }}
            >
              <Archive size={28} style={{ color: "oklch(0.85 0.05 50)" }} />
            </div>
            <p class="text-embossed-on-dark text-sm font-semibold">No libraries yet</p>
            <p class="mt-1 text-sm" style={{ color: "oklch(0.85 0.05 50)" }}>
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
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            <For each={libraries()}>
              {(lib) => {
                const isActive = () => uiStore.activeLibrary?.path === lib.path;
                const isExporting = () => exportingPath() === lib.path;
                const isDeleting = () => deletingPath() === lib.path;
                const isBusy = () => isExporting() || isDeleting();
                return (
                  <div
                    role="button"
                    tabindex="0"
                    class="group cursor-pointer p-4 transition-all duration-200 outline-none hover:translate-y-[-2px]"
                    classList={{
                      "surface-pressed": isActive(),
                      "material-paper border-stitched": !isActive(),
                      "opacity-60 pointer-events-none": isDeleting(),
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
                    <div class="flex flex-wrap gap-2" onClick={(e) => e.stopPropagation()}>
                      <button
                        onClick={() => handleExport(lib)}
                        disabled={isBusy()}
                        class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
                      >
                        <Show when={isExporting()} fallback={<Archive size={12} />}>
                          <Loader2 size={12} class="animate-spin" />
                        </Show>
                        Export ZIP
                      </button>
                      <button
                        onClick={() => api.revealInFinder(lib.path)}
                        disabled={isBusy()}
                        class="btn-tactile flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium"
                      >
                        <FolderOpen size={12} />
                        Show
                      </button>
                      <button
                        onClick={() => handleDelete(lib)}
                        disabled={isBusy()}
                        title="Delete library"
                        class="btn-tactile ml-auto flex items-center gap-1 px-2.5 py-1.5 text-xs font-medium"
                        style={{ color: "var(--color-destructive)" }}
                      >
                        <Show when={isDeleting()} fallback={<Trash2 size={12} />}>
                          <Loader2 size={12} class="animate-spin" />
                        </Show>
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
