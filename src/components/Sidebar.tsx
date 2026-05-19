import { For, Show, createMemo } from "solid-js";
import { Compass, BookOpen, Settings as SettingsIcon } from "lucide-solid";
import { uiStore } from "@/stores/ui";
import { runStore } from "@/stores/run";
import { formatBytes } from "@/lib/utils";

export default function Sidebar() {
  const recent = createMemo(() => uiStore.knownLibraries.slice(0, 5));
  const libCount = () => uiStore.knownLibraries.length;
  const running = () => runStore.state.running;

  return (
    <nav class="df-sidebar" aria-label="Primary">
      <div class="df-brand">
        <div class="df-brand-mark">Df</div>
        <div class="df-brand-name">Document Finder</div>
      </div>

      <div class="df-nav">
        <button
          class="df-nav-item"
          aria-current={uiStore.view === "find" ? "page" : undefined}
          onClick={() => uiStore.setView("find")}
        >
          <Compass size={15} />
          <span>Discover</span>
          <Show when={running()}>
            <span class="df-nav-count df-nav-count-live">
              <span class="df-pulse" style={{ width: "6px", height: "6px" }} /> live
            </span>
          </Show>
        </button>
        <button
          class="df-nav-item"
          aria-current={uiStore.view === "library" ? "page" : undefined}
          onClick={() => uiStore.setView("library")}
        >
          <BookOpen size={15} />
          <span>Library</span>
          <Show when={libCount() > 0}>
            <span class="df-nav-count">{libCount()}</span>
          </Show>
        </button>
        <button
          class="df-nav-item"
          aria-current={uiStore.view === "settings" ? "page" : undefined}
          onClick={() => uiStore.setView("settings")}
        >
          <SettingsIcon size={15} />
          <span>Settings</span>
        </button>
      </div>

      <Show when={recent().length > 0}>
        <div class="df-side-section">
          <span>Recent</span>
          <button
            class="df-btn ghost sm"
            style={{ padding: "2px 6px" }}
            onClick={() => uiStore.setView("library")}
            title="See all libraries"
          >
            all
          </button>
        </div>
        <div class="df-recent">
          <For each={recent()}>{(lib) => (
            <button
              class="df-recent-item"
              classList={{ active: uiStore.activeLibrary?.path === lib.path }}
              onClick={() => {
                uiStore.setActiveLibrary(lib);
                uiStore.setView("library");
              }}
              title={lib.query ?? lib.name}
            >
              <span class="df-recent-title">{lib.query ?? lib.name}</span>
              <span class="df-recent-meta">
                {lib.n_docs} docs · {formatBytes(lib.size_bytes)}
              </span>
            </button>
          )}</For>
        </div>
      </Show>

      <div class="df-side-footer">
        <span class="df-status-dot" /> Backend ready
      </div>
    </nav>
  );
}
