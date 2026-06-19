import { Show, For, createEffect } from "solid-js";
import { Compass, Library as LibraryIcon, Settings as SettingsIcon } from "lucide-solid";
import Logo from "@/components/Logo";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { runStore } from "@/stores/run";
import { api } from "@/lib/tauri";
import { compareLibraryRecency, formatBytes } from "@/lib/utils";

const navItems = [
  { id: "find" as const, label: "Discover", icon: Compass },
  { id: "library" as const, label: "Library", icon: LibraryIcon },
  { id: "settings" as const, label: "Settings", icon: SettingsIcon },
];

export default function Sidebar() {
  // Lazy-prime the library list so the count + Recent section show real data on
  // first paint if libraries already exist on disk. A createEffect (not onMount)
  // so it re-fires when settings.libraryRoot arrives ASYNCHRONOUSLY — on a fresh
  // launch the root starts "" and is filled in once defaultLibraryDir() resolves.
  // Guarded by a one-shot-per-root latch (NOT knownLibraries.length): an empty
  // result writes a fresh [] reference which would re-trigger an effect that read
  // .length, busy-looping list_libraries IPC forever on a library-less install.
  let primedRoot = "";
  createEffect(() => {
    const root = settings.libraryRoot;
    if (root && root !== primedRoot) {
      primedRoot = root;
      api
        .listLibraries(root)
        .then((libs) => uiStore.setKnownLibraries(libs))
        .catch(() => {
          primedRoot = ""; // allow a retry if the fetch failed
        });
    }
  });

  const focusMain = () =>
    requestAnimationFrame(() => (document.getElementById("main-content") as HTMLElement)?.focus());

  const recent = () =>
    [...uiStore.knownLibraries].sort((a, b) => compareLibraryRecency(a.name, b.name)).slice(0, 5);

  return (
    <nav class="df-sidebar" aria-label="Primary">
      <div class="df-brand">
        <Logo size={28} class="df-brand-logo" />
        <span class="df-brand-name">Document Finder</span>
      </div>

      <div class="df-nav">
        <For each={navItems}>
          {(item) => (
            <button
              class="df-nav-item"
              aria-current={uiStore.view === item.id ? "page" : undefined}
              onClick={() => {
                uiStore.setView(item.id);
                focusMain();
              }}
            >
              <item.icon size={15} />
              {item.label}
              <Show when={item.id === "find" && runStore.state.running}>
                <span
                  class="df-nav-count"
                  style={{
                    color: "var(--accent)",
                    display: "flex",
                    "align-items": "center",
                    gap: "4px",
                  }}
                >
                  <span class="df-pulse" style={{ width: "6px", height: "6px" }} /> live
                </span>
              </Show>
              <Show when={item.id === "library" && uiStore.lifetimeStats.count > 0}>
                <span class="df-nav-count">{uiStore.lifetimeStats.count}</span>
              </Show>
            </button>
          )}
        </For>
      </div>

      <Show when={recent().length > 0}>
        <div class="df-side-section">
          Recent
          <button
            class="df-btn ghost sm"
            style={{ padding: "2px 6px", "margin-right": "-6px" }}
            onClick={() => {
              uiStore.setView("library");
              focusMain();
            }}
            title="See all libraries"
          >
            all
          </button>
        </div>
        <div class="df-recent">
          <For each={recent()}>
            {(lib) => (
              <button
                classList={{
                  "df-recent-item": true,
                  active: uiStore.activeLibrary?.path === lib.path,
                }}
                onClick={() => {
                  uiStore.setActiveLibrary(lib);
                  uiStore.setView("library");
                  focusMain();
                }}
              >
                <span class="df-recent-title">{lib.query || lib.name}</span>
                <span class="df-recent-meta">
                  {lib.n_docs} docs · {formatBytes(lib.size_bytes)}
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>

      <div class="df-side-footer">
        <Show
          when={uiStore.listenersReady}
          fallback={
            <>
              <span class="df-status-dot warn" /> Live updates unavailable — restart
            </>
          }
        >
          <span class="df-status-dot" /> Backend ready
        </Show>
      </div>
    </nav>
  );
}
