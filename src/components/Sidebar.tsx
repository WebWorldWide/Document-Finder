import { For, Show, createEffect, on } from "solid-js";
import { Library, Search, Settings } from "lucide-solid";
import Logo from "@/components/Logo";
import { api } from "@/lib/tauri";
import { compareLibraryRecency, formatRelativeTime, libraryTimestamp } from "@/lib/utils";
import { runStore } from "@/stores/run";
import { settings } from "@/stores/settings";
import { uiStore, type View } from "@/stores/ui";

const NAV: { id: View; label: string; icon: typeof Search }[] = [
  { id: "discover", label: "Discover", icon: Search },
  { id: "library", label: "Library", icon: Library },
  { id: "settings", label: "Settings", icon: Settings },
];

export async function refreshLibraries() {
  if (!settings.libraryRoot) return;
  try {
    const libs = await api.listLibraries(settings.libraryRoot);
    uiStore.setKnownLibraries(libs);
  } catch (e) {
    console.error("listLibraries failed", e);
  }
}

export default function Sidebar() {
  createEffect(
    on(
      () => settings.libraryRoot,
      () => void refreshLibraries(),
    ),
  );
  // A finished run adds/updates a library.
  createEffect(
    on(
      () => runStore.state.status,
      (s) => {
        if (s === "done" || s === "cancelled") void refreshLibraries();
      },
    ),
  );

  const recent = () =>
    [...uiStore.knownLibraries].sort((a, b) => compareLibraryRecency(a.name, b.name)).slice(0, 6);

  const go = (v: View) => {
    uiStore.setView(v);
    queueMicrotask(() => document.getElementById("main-content")?.focus());
  };

  return (
    <nav class="df-side" aria-label="Main">
      <div class="df-brand">
        <Logo size={26} />
        <span class="df-brand-name df-side-text">Document Finder</span>
      </div>

      <div class="df-nav">
        <For each={NAV}>
          {(item) => (
            <button
              class="df-nav-item"
              aria-current={uiStore.view === item.id ? "page" : undefined}
              aria-label={item.label}
              title={item.label}
              onClick={() => go(item.id)}
            >
              <item.icon size={16} stroke-width={1.75} aria-hidden="true" />
              <span class="df-side-text">{item.label}</span>
              <Show when={item.id === "discover" && runStore.live}>
                <span class="df-live-dot" aria-label="Search running" />
              </Show>
              <Show when={item.id === "library" && uiStore.knownLibraries.length > 0}>
                <span class="df-nav-badge num df-side-text">{uiStore.knownLibraries.length}</span>
              </Show>
            </button>
          )}
        </For>
      </div>

      <Show when={recent().length > 0 && uiStore.view !== "library"}>
        <div class="df-recent">
          <div class="df-label">Recent</div>
          <For each={recent()}>
            {(lib) => {
              const ts = libraryTimestamp(lib.name);
              return (
                <button class="df-recent-item" onClick={() => uiStore.showLibrary(lib.path)}>
                  <b>{lib.query}</b>
                  <span class="num">
                    {lib.n_docs} docs
                    {ts ? ` · ${formatRelativeTime(ts)}` : ""}
                  </span>
                </button>
              );
            }}
          </For>
        </div>
      </Show>

      <Show when={!uiStore.listenersReady}>
        <div class="df-side-foot" role="status">
          Live updates are unavailable — restart the app.
        </div>
      </Show>
    </nav>
  );
}
