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
import { runStore } from "@/stores/run";
import { compareLibraryRecency, formatBytes } from "@/lib/utils";
import Banner from "./Banner";

type SortKey = "updated" | "docs" | "size";

export default function LibraryView() {
  const [libraries, setLibraries] = createSignal<LibraryInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [exportingPath, setExportingPath] = createSignal<string | null>(null);
  const [deletingPath, setDeletingPath] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [exportOk, setExportOk] = createSignal<{
    dest: string;
    files: number;
    bytes: number;
  } | null>(null);
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

  // Refresh the grid when a run finishes while the Library tab is open — without
  // this, a search completing in the background leaves the grid showing pre-run
  // data with no manual refresh control.
  let wasRunning = runStore.state.running;
  createEffect(() => {
    const running = runStore.state.running;
    if (wasRunning && !running) setLoadTick((n) => n + 1);
    wasRunning = running;
  });

  const sorted = createMemo(() => {
    let xs = libraries().slice();
    const q = filter().trim().toLowerCase();
    if (q) xs = xs.filter((l) => (l.query ?? l.name).toLowerCase().includes(q));
    if (sortBy() === "size") xs.sort((a, b) => b.size_bytes - a.size_bytes);
    else if (sortBy() === "docs") xs.sort((a, b) => b.n_docs - a.n_docs);
    // Default "updated"/Recent: order by the run timestamp embedded in the
    // folder name — the backend's reverse-name order is slug-dominated and isn't
    // actually by recency.
    else xs.sort((a, b) => compareLibraryRecency(a.name, b.name));
    return xs;
  });

  async function handleExport(lib: LibraryInfo) {
    setActionError(null);
    setExportOk(null);
    const dest = await save({
      defaultPath: `${lib.name}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!dest) return;
    setExportingPath(lib.path);
    try {
      const result = await api.exportLibraryZip(lib.path, dest);
      // Confirm success on-screen — reveal-in-Finder is best-effort and can do
      // nothing visible (sandbox / headless Linux), leaving the user unsure the
      // .zip was written.
      setExportOk({ dest: result.dest, files: result.files, bytes: result.size_bytes });
      uiStore.announce(
        `Exported ${result.files} file${result.files === 1 ? "" : "s"} to ${result.dest}`,
      );
      // Reveal is a best-effort follow-up — a failure to open the OS file browser
      // must NOT be reported as an export failure (the .zip is already written).
      api.revealInFinder(result.dest).catch(() => {});
    } catch (e) {
      setActionError(`Couldn't export this library: ${String(e)}`);
      uiStore.announce(`Export failed: ${String(e)}`);
    } finally {
      setExportingPath(null);
    }
  }

  async function handleDelete(lib: LibraryInfo) {
    setActionError(null);
    setExportOk(null); // clear a stale "Exported …" banner from a prior action
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
      // Optimistically drop the card NOW so it can't be re-clicked (Delete/Export
      // on an already-gone path) during the async refetch window; clear the
      // deleting latch too since the card is gone. The refetch reconciles disk.
      setLibraries((xs) => xs.filter((l) => l.path !== lib.path));
      setDeletingPath(null);
      setLoadTick((n) => n + 1);
    } catch (e) {
      setActionError(`Couldn't delete this library: ${String(e)}`);
      // Only re-enable the card on failure (it still exists).
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
                <button
                  class={sortBy() === k ? "on" : ""}
                  aria-pressed={sortBy() === k}
                  onClick={() => setSortBy(k)}
                >
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

        <Show when={exportOk()}>
          {(ok) => (
            <div style={{ "margin-bottom": "16px" }}>
              <Banner kind="ok" onDismiss={() => setExportOk(null)}>
                Exported {ok().files} file{ok().files === 1 ? "" : "s"} ({formatBytes(ok().bytes)})
                to <code>{ok().dest}</code>
              </Banner>
            </div>
          )}
        </Show>

        {/* Full-screen spinner ONLY on the first load (empty list). Background
            refreshes (run-finished, post-delete) keep the existing grid mounted
            so it doesn't flash blank — which read as "did my libraries vanish?". */}
        <Show when={loading() && libraries().length === 0}>
          <div
            role="status"
            aria-label="Loading your libraries"
            style={{ display: "flex", "justify-content": "center", padding: "64px 0" }}
          >
            <Loader2 size={24} class="spin" style={{ color: "var(--ink-3)" }} />
          </div>
        </Show>

        <Show when={!loading() && error()}>
          <div style={{ "margin-bottom": "16px" }}>
            <Banner kind="bad">
              <div>
                {/* If we already have a list on screen, a refetch failure is
                    non-destructive — keep the grid and say so, rather than wiping
                    the user's libraries off the screen. */}
                {libraries().length > 0
                  ? "Couldn't refresh — showing your last loaded libraries."
                  : "We couldn't read your library folder."}{" "}
                {error()}
                <div style={{ "margin-top": "8px" }}>
                  <button class="df-btn sm" onClick={() => setLoadTick((n) => n + 1)}>
                    <RefreshCw size={12} /> Retry
                  </button>
                </div>
              </div>
            </Banner>
          </div>
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

        {/* Grid stays mounted during a background refresh (no !loading gate) and
            hides on error so a stale grid can't render under the error banner. */}
        {/* Grid stays mounted even on a refetch error (the banner above explains
            it) — only the empty-list error state hides it. */}
        <Show when={sorted().length > 0}>
          <div class="df-libgrid">
            <For each={sorted()}>
              {(lib) => {
                const isActive = () => uiStore.activeLibrary?.path === lib.path;
                const isExporting = () => exportingPath() === lib.path;
                const isDeleting = () => deletingPath() === lib.path;
                const isBusy = () => isExporting() || isDeleting();
                return (
                  // Plain card, not a role=button: there is no in-app detail view
                  // for a selected library, so a card-level click had no real
                  // effect AND its keydown handler swallowed Enter/Space on the
                  // inner Export/Show/Delete buttons. The explicit buttons are the
                  // interactive elements.
                  <div
                    classList={{
                      "df-libcard": true,
                      active: isActive(),
                      "opacity-60 pointer-events-none": isDeleting(),
                    }}
                  >
                    <div class="df-libcard-head">
                      <div class="df-libcard-q" title={lib.query ?? lib.name}>
                        {lib.query ?? lib.name}
                      </div>
                    </div>
                    <div class="df-libcard-meta">
                      <span>
                        <strong>{lib.n_docs}</strong> document{lib.n_docs === 1 ? "" : "s"}
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
                        onClick={() =>
                          api
                            .revealInFinder(lib.path)
                            .catch((e) =>
                              setActionError(`Couldn't open this library's folder: ${String(e)}`),
                            )
                        }
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
