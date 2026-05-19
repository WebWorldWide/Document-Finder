import { createSignal, createEffect, onCleanup, Show, For, createMemo } from "solid-js";
import { Archive, FolderOpen, Loader2, X, RefreshCw, BookOpen } from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import { api, type LibraryInfo } from "@/lib/tauri";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { log } from "@/lib/log";
import { formatBytes } from "@/lib/utils";

export default function LibraryView() {
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [exportingPath, setExportingPath] = createSignal<string | null>(null);
  const [exportError, setExportError] = createSignal<string | null>(null);
  const [loadTick, setLoadTick] = createSignal(0);

  const libraries = createMemo(() => uiStore.knownLibraries);
  const totalDocs = createMemo(() =>
    libraries().reduce((s, l) => s + l.n_docs, 0),
  );
  const totalBytes = createMemo(() =>
    libraries().reduce((s, l) => s + l.size_bytes, 0),
  );

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
          uiStore.setKnownLibraries(libs);
          setLoading(false);
          log.info("library", `listed ${libs.length} libraries`);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
          log.error("library", "listLibraries failed", e);
        }
      });
    onCleanup(() => { cancelled = true; });
  });

  async function handleExport(lib: LibraryInfo, e: MouseEvent) {
    e.stopPropagation();
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
      log.info("library", `exported ${lib.name} to ${result.dest}`, {
        files: result.files,
        bytes: result.size_bytes,
      });
    } catch (e) {
      setExportError(String(e));
      log.error("library", `export ${lib.name} failed`, e);
    } finally {
      setExportingPath(null);
    }
  }

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <h1 class="df-canvas-title">Library</h1>
        <div class="df-headstats">
          <div class="df-headstat">
            <span class="df-headstat-num">{libraries().length}</span>
            <span class="df-headstat-label">libraries</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{totalDocs()}</span>
            <span class="df-headstat-label">docs saved</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{formatBytes(totalBytes())}</span>
            <span class="df-headstat-label">on disk</span>
          </div>
        </div>
      </div>

      <div class="df-canvas-body" style={{ "padding-top": "var(--pad-6)" }}>
        <Show when={exportError()}>
          <div class="df-banner bad" style={{ "margin-bottom": "var(--pad-4)" }}>
            <X size={14} />
            <div class="df-banner-body">
              <strong>Export failed.</strong> {exportError()}
            </div>
            <button
              class="df-banner-x"
              onClick={() => setExportError(null)}
              aria-label="Dismiss"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        <Show when={loading()}>
          <div style={{
            display: "flex",
            "justify-content": "center",
            padding: "var(--pad-9) 0",
          }}>
            <Loader2 size={24} class="spin" style={{ color: "var(--ink-3)" }} />
          </div>
        </Show>

        <Show when={!loading() && error()}>
          <div class="df-banner bad">
            <X size={14} />
            <div class="df-banner-body">
              <strong>Could not list libraries.</strong> {error()}
            </div>
            <button
              class="df-btn sm"
              onClick={() => setLoadTick((n) => n + 1)}
              style={{ "flex-shrink": 0 }}
            >
              <RefreshCw size={12} /> Retry
            </button>
          </div>
        </Show>

        <Show when={!loading() && !error() && libraries().length === 0}>
          <div class="df-empty">
            <div class="df-empty-mark">
              <BookOpen size={28} />
            </div>
            <h3 class="df-empty-title">No libraries yet</h3>
            <p class="df-empty-sub">
              Run a search from Discover and saved documents will land here as a
              library collection.
            </p>
            <button class="df-btn accent" onClick={() => uiStore.setView("find")}>
              Go to Discover
            </button>
          </div>
        </Show>

        <Show when={!loading() && libraries().length > 0}>
          <div class="df-libgrid">
            <For each={libraries()}>{(lib) => {
              const isActive = () => uiStore.activeLibrary?.path === lib.path;
              const isExporting = () => exportingPath() === lib.path;
              return (
                <div
                  class="df-libcard"
                  classList={{ active: isActive() }}
                  role="button"
                  tabindex="0"
                  onClick={() => uiStore.setActiveLibrary(lib)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      uiStore.setActiveLibrary(lib);
                    }
                  }}
                >
                  <div class="df-libcard-head">
                    <span class="df-libcard-q" title={lib.query ?? lib.name}>
                      {lib.query ?? lib.name}
                    </span>
                  </div>
                  <div class="df-libcard-meta">
                    <span><strong>{lib.n_docs}</strong> docs</span>
                    <span><strong>{formatBytes(lib.size_bytes)}</strong></span>
                  </div>
                  <div class="df-libcard-actions" onClick={(e) => e.stopPropagation()}>
                    <button
                      class="df-btn sm"
                      onClick={(e) => handleExport(lib, e)}
                      disabled={isExporting()}
                    >
                      <Show when={isExporting()} fallback={<Archive size={12} />}>
                        <Loader2 size={12} class="spin" />
                      </Show>
                      Export ZIP
                    </button>
                    <button
                      class="df-btn sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        api.revealInFinder(lib.path);
                      }}
                    >
                      <FolderOpen size={12} /> Show
                    </button>
                  </div>
                </div>
              );
            }}</For>
          </div>
        </Show>
      </div>
    </div>
  );
}
