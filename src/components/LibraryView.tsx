import { createSignal, createEffect, createMemo, onCleanup, Show, For } from "solid-js";
import {
  Archive,
  FolderOpen,
  Loader2,
  RefreshCw,
  Trash2,
  Search,
  Library as LibraryIcon,
} from "lucide-solid";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { api, type LibraryInfo } from "@/lib/tauri";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { formatBytes } from "@/lib/utils";
import Banner from "./Banner";

type SortKey = "updated" | "docs" | "size";

export default function LibraryView() {
  const [libraries, setLibraries] = createSignal<LibraryInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [exportingPath, setExportingPath] = createSignal<string | null>(null);
  const [deletingPath, setDeletingPath] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [loadTick, setLoadTick] = createSignal(0);
  const [filter, setFilter] = createSignal("");
  const [sortBy, setSortBy] = createSignal<SortKey>("updated");

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

  const sorted = createMemo(() => {
    let xs = libraries().slice();
    const q = filter().trim().toLowerCase();
    if (q) xs = xs.filter((l) => (l.query ?? l.name).toLowerCase().includes(q));
    if (sortBy() === "size") xs.sort((a, b) => b.size_bytes - a.size_bytes);
    else if (sortBy() === "docs") xs.sort((a, b) => b.n_docs - a.n_docs);
    return xs;
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
      setActionError(`Couldn't export this library: ${String(e)}`);
    } finally {
      setExportingPath(null);
    }
  }

  async function handleDelete(lib: LibraryInfo) {
    setActionError(null);
    const ok = await ask(
      `Delete library "${lib.query ?? lib.name}"? This permanently removes ${lib.n_docs} document${
        lib.n_docs === 1 ? "" : "s"
      } and ${formatBytes(lib.size_bytes)} from disk. This cannot be undone.`,
      { title: "Delete Library", kind: "warning" },
    );
    if (!ok) return;
    if (uiStore.activeLibrary?.path === lib.path) uiStore.setActiveLibrary(null);
    setDeletingPath(lib.path);
    try {
      await api.deleteLibrary(lib.path);
      setLoadTick((n) => n + 1);
    } catch (e) {
      setActionError(`Couldn't delete this library: ${String(e)}`);
    } finally {
      setDeletingPath(null);
    }
  }

  const SORTS: [SortKey, string][] = [
    ["updated", "Recent"],
    ["docs", "Docs"],
    ["size", "Size"],
  ];

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <div>
          <div class="df-eyebrow">Library</div>
          <h1 class="df-canvas-title">Library</h1>
        </div>
        <div style={{ display: "flex", gap: "8px", "align-items": "center" }}>
          <div class="df-inline-search">
            <Search size={13} style={{ color: "var(--ink-3)" }} />
            <input
              type="text"
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
              placeholder="Filter libraries…"
              aria-label="Filter libraries"
            />
          </div>
          <div class="df-seg">
            <For each={SORTS}>
              {([k, label]) => (
                <button class={sortBy() === k ? "on" : ""} onClick={() => setSortBy(k)}>
                  {label}
                </button>
              )}
            </For>
          </div>
        </div>
      </div>

      <div class="df-canvas-body" style={{ "padding-top": "20px" }}>
        <Show when={actionError()}>
          <div style={{ "margin-bottom": "16px" }}>
            <Banner kind="bad" onDismiss={() => setActionError(null)}>
              {actionError()}
            </Banner>
          </div>
        </Show>

        <Show when={loading()}>
          <div style={{ display: "flex", "justify-content": "center", padding: "64px 0" }}>
            <Loader2 size={24} class="spin" style={{ color: "var(--ink-3)" }} />
          </div>
        </Show>

        <Show when={!loading() && error()}>
          <Banner kind="bad">
            <div>
              We couldn&rsquo;t read your library folder. {error()}
              <div style={{ "margin-top": "8px" }}>
                <button class="df-btn sm" onClick={() => setLoadTick((n) => n + 1)}>
                  <RefreshCw size={12} /> Retry
                </button>
              </div>
            </div>
          </Banner>
        </Show>

        <Show when={!loading() && !error() && sorted().length === 0}>
          <div class="df-empty">
            <div class="df-empty-mark">
              <LibraryIcon size={28} />
            </div>
            <h2 class="df-empty-title">{filter() ? "No matches" : "No libraries yet"}</h2>
            <p class="df-empty-sub">
              {filter()
                ? `Nothing in your library matches “${filter()}”.`
                : "Run a search and Document Finder saves everything it downloads into its own collection here."}
            </p>
            <button
              class="df-btn accent"
              onClick={() => (filter() ? setFilter("") : uiStore.setView("find"))}
            >
              {filter() ? "Clear filter" : "Go to Discover"}
            </button>
          </div>
        </Show>

        <Show when={!loading() && sorted().length > 0}>
          <div class="df-libgrid">
            <For each={sorted()}>
              {(lib) => {
                const isActive = () => uiStore.activeLibrary?.path === lib.path;
                const isExporting = () => exportingPath() === lib.path;
                const isDeleting = () => deletingPath() === lib.path;
                const isBusy = () => isExporting() || isDeleting();
                return (
                  <div
                    role="button"
                    tabindex="0"
                    classList={{
                      "df-libcard": true,
                      active: isActive(),
                      "opacity-60 pointer-events-none": isDeleting(),
                    }}
                    onClick={() => uiStore.setActiveLibrary(lib)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        uiStore.setActiveLibrary(lib);
                      }
                    }}
                  >
                    <div class="df-libcard-head">
                      <div class="df-libcard-q" title={lib.query ?? lib.name}>
                        {lib.query ?? lib.name}
                      </div>
                    </div>
                    <div class="df-libcard-meta">
                      <span>
                        <strong>{lib.n_docs}</strong> documents
                      </span>
                      <span style={{ color: "var(--ink-4)" }}>·</span>
                      <span>{formatBytes(lib.size_bytes)}</span>
                    </div>
                    <div
                      class="df-libcard-actions"
                      onClick={(e) => {
                        e.stopPropagation();
                      }}
                    >
                      <button
                        class="df-btn sm"
                        onClick={() => handleExport(lib)}
                        disabled={isBusy()}
                      >
                        <Show when={isExporting()} fallback={<Archive size={12} />}>
                          <Loader2 size={12} class="spin" />
                        </Show>
                        Export
                      </button>
                      <button
                        class="df-btn sm ghost"
                        onClick={() => api.revealInFinder(lib.path)}
                        disabled={isBusy()}
                      >
                        <FolderOpen size={12} /> Show
                      </button>
                      <span style={{ flex: 1 }} />
                      <button
                        class="df-btn sm danger"
                        onClick={() => handleDelete(lib)}
                        disabled={isBusy()}
                        title="Delete library"
                        aria-label={`Delete ${lib.query ?? lib.name}`}
                      >
                        <Show when={isDeleting()} fallback={<Trash2 size={12} />}>
                          <Loader2 size={12} class="spin" />
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
