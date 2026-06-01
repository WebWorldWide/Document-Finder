import { Show, For, onMount } from "solid-js";
import { Compass, Library as LibraryIcon, Settings as SettingsIcon } from "lucide-solid";
import Logo from "@/components/Logo";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { runStore } from "@/stores/run";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

const navItems = [
  { id: "find" as const, label: "Discover", icon: Compass },
  { id: "library" as const, label: "Library", icon: LibraryIcon },
  { id: "settings" as const, label: "Settings", icon: SettingsIcon },
];

export default function Sidebar() {
  // Lazy-prime the library list so the count + Recent section show real data
  // on first paint if libraries already exist on disk.
  onMount(() => {
    if (uiStore.knownLibraries.length === 0 && settings.libraryRoot) {
      api
        .listLibraries(settings.libraryRoot)
        .then((libs) => uiStore.setKnownLibraries(libs))
        .catch(() => {});
    }
  });

  const focusMain = () =>
    requestAnimationFrame(() => (document.getElementById("main-content") as HTMLElement)?.focus());

  const recent = () => uiStore.knownLibraries.slice(0, 5);

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
        <span class="df-status-dot" /> Backend ready
      </div>
    </nav>
  );
}
